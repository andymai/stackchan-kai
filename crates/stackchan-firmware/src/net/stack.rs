//! embassy-net runner task. Drives the TCP/IP stack on top of the
//! esp-radio station-mode `WifiDevice` and runs `DHCPv4` once the
//! link is up.

use embassy_net::{Runner, StackResources};
use esp_radio::wifi::WifiDevice;
use static_cell::StaticCell;

/// Maximum number of concurrent sockets the firmware ever needs.
///
/// `smoltcp` panics ("adding a socket to a full `SocketSet`") the moment
/// demand exceeds this, so it must cover every consumer that can be live at
/// once: `HTTP_WORKER_COUNT` HTTP listeners (each owns its own socket), plus
/// mDNS, SNTP, the DHCP + DNS clients, and the optional outbound clients
/// (agent sidecar, `VoiceVox`, audio debug). The old value of 6 pre-dated the
/// 4-worker HTTP pool and overflowed as soon as Wi-Fi linked. Slots are cheap
/// static metadata, so over-provisioning costs only a few `.bss` bytes.
pub const STACK_SOCKETS: usize = crate::net::http::HTTP_WORKER_COUNT + 8;

/// Static cell for the embassy-net resource pool. Sized at compile
/// time to avoid heap fragmentation under SNTP/HTTP/mDNS churn.
pub static STACK_RESOURCES: StaticCell<StackResources<STACK_SOCKETS>> = StaticCell::new();

/// embassy-net runner task. Spins forever processing the network
/// stack's internal queues; reading or writing on the corresponding
/// `Stack` borrows from the same instance.
#[embassy_executor::task]
pub async fn net_runner_task(mut runner: Runner<'static, WifiDevice<'static>>) -> ! {
    runner.run().await
}
