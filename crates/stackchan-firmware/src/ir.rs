//! IR receiver task: RMT RX → NEC decoder → Signal pipeline.
//!
//! Wraps an esp-hal RMT async RX channel, decodes the pulse train
//! via the [`ir_nec`] crate, and publishes [`NecCommand`] values on
//! [`REMOTE_SIGNAL`]. The render task drains the signal and feeds
//! each command into the [`stackchan_core::modifiers::RemoteCommand`]
//! modifier.
//!
//! ## CoreS3 pin caveat
//!
//! The memory cheat-sheet doesn't list the IR receiver's GPIO, so
//! [`IR_RX_PIN`] is a best-guess `GPIO21` — the pin most commonly used
//! on M5Stack boards for IR RX. Flash the firmware and, if no IR
//! frames decode even with a working remote + `ir-bench`, update
//! the const against the actual CoreS3 schematic.
//!
//! ## Signal polarity
//!
//! IRM56384-class receivers output **active-low** (IR burst present
//! → GPIO goes low). The `ir_nec` decoder expects `level = true` to
//! mean "mark" (IR burst). This task inverts the RMT level bits on
//! conversion so the decoder sees the logical polarity it expects.

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::Level,
    peripherals::{GPIO21, RMT},
    rmt::{PulseCode, Rmt, RxChannelConfig, RxChannelCreator},
    time::Rate,
};
use ir_nec::{NecCommand, Pulse};

/// Raw NEC commands decoded from the IR receiver: IR task → render task.
///
/// The render task drains via [`Signal::try_take`] each frame and calls
/// [`stackchan_core::modifiers::RemoteCommand::queue`] with the
/// `(address, command)` pair. Signal semantics (latest wins, no backlog)
/// mean if the user mashes buttons between render ticks, only the most
/// recent command actually acts — acceptable for an emotion-control UX.
pub static REMOTE_SIGNAL: Signal<CriticalSectionRawMutex, NecCommand> = Signal::new();

/// Size of the RMT RX buffer, measured in [`PulseCode`] units.
/// A full NEC frame is 67 pulse *edges* = 34 pulse codes. 64 gives us
/// headroom for the stop bit + a repeat pulse without allocation.
const RX_BUFFER_LEN: usize = 64;

/// RMT idle threshold in ticks (with the 1 µs tick rate set below,
/// this is microseconds). NEC's end-of-frame gap is ~40 ms, so
/// anything longer than ~10 ms marks a complete frame.
const RX_IDLE_THRESHOLD: u16 = 10_000;

/// RMT filter threshold in ticks. Pulses shorter than this are
/// treated as noise. 50 µs is well under NEC's 560 µs bit-mark.
const RX_FILTER_THRESHOLD: u8 = 50;

/// Clock divider for the RMT peripheral. At 80 MHz RMT base clock,
/// `divider = 80` gives 1 µs per tick, which matches the
/// `duration_us` units `ir-nec` expects.
const RX_CLK_DIVIDER: u8 = 80;

/// Configure the RMT RX channel and decode NEC frames forever.
///
/// Takes the RMT peripheral + the IR-receiver GPIO pin. Bus errors
/// at configure-time log at `error` and park the task — silent
/// degradation matches the other optional-peripheral tasks
/// (`ambient`, `imu`, `button`).
pub async fn run_ir_loop(rmt_peripheral: RMT<'static>, ir_rx_pin: GPIO21<'static>) -> ! {
    let rmt = match Rmt::new(rmt_peripheral, Rate::from_mhz(80)) {
        Ok(r) => r.into_async(),
        Err(e) => {
            defmt::error!(
                "IR: RMT init failed ({}); IR-driven behaviors disabled",
                defmt::Debug2Format(&e),
            );
            park().await;
        }
    };

    let rx_config = RxChannelConfig::default()
        .with_clk_divider(RX_CLK_DIVIDER)
        .with_idle_threshold(RX_IDLE_THRESHOLD)
        .with_filter_threshold(RX_FILTER_THRESHOLD);
    let mut channel = match rmt.channel7.configure_rx(ir_rx_pin, rx_config) {
        Ok(c) => c,
        Err(e) => {
            defmt::error!(
                "IR: RMT RX channel configure failed ({}); IR-driven behaviors disabled",
                defmt::Debug2Format(&e),
            );
            park().await;
        }
    };
    defmt::info!(
        "IR: RMT RX on channel7 pin {=str} (1 us tick, idle {=u16} us, filter {=u8} us)",
        "GPIO21 (TBD vs CoreS3 schematic)",
        RX_IDLE_THRESHOLD,
        RX_FILTER_THRESHOLD,
    );

    // Reusable pulse-code + pulse buffers. `RX_BUFFER_LEN * 2` is the
    // max number of logical pulses we can decode per transaction
    // because each PulseCode packs two pulse halves.
    let mut codes = [PulseCode::default(); RX_BUFFER_LEN];
    let mut pulses = [Pulse {
        level: false,
        duration_us: 0,
    }; RX_BUFFER_LEN * 2];

    loop {
        match channel.receive(&mut codes).await {
            Ok(valid_codes) => {
                let pulse_count = pulse_codes_to_pulses(&codes[..valid_codes], &mut pulses);
                if let Some(cmd) = ir_nec::decode(&pulses[..pulse_count]) {
                    defmt::info!(
                        "IR: decoded addr=0x{=u16:04X} cmd=0x{=u8:02X}",
                        cmd.address,
                        cmd.command,
                    );
                    REMOTE_SIGNAL.signal(cmd);
                } else {
                    defmt::debug!(
                        "IR: {=usize} pulses, decode failed (noise / unknown protocol)",
                        pulse_count,
                    );
                }
            }
            Err(e) => defmt::warn!("IR: RMT receive failed: {}", defmt::Debug2Format(&e)),
        }
    }
}

/// Convert a slice of RMT [`PulseCode`]s into the [`Pulse`] array the
/// decoder expects, inverting level polarity for active-low IR
/// receivers.
///
/// Stops at the first pulse code with `length1 == 0` (RMT's
/// end-of-data sentinel) OR when the destination buffer is full.
/// Returns the number of [`Pulse`]s written.
fn pulse_codes_to_pulses(codes: &[PulseCode], pulses: &mut [Pulse]) -> usize {
    let mut out = 0;
    for code in codes {
        if code.length1() == 0 {
            break;
        }
        if out >= pulses.len() {
            break;
        }
        pulses[out] = Pulse {
            // IR receiver is active-low: PulseCode::Low == IR burst.
            level: code.level1() == Level::Low,
            duration_us: u32::from(code.length1()),
        };
        out += 1;

        if code.length2() == 0 {
            break;
        }
        if out >= pulses.len() {
            break;
        }
        pulses[out] = Pulse {
            level: code.level2() == Level::Low,
            duration_us: u32::from(code.length2()),
        };
        out += 1;
    }
    out
}

/// Idle loop for the post-failure path.
async fn park() -> ! {
    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}
