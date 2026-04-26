//! embassy-net runner task. Drives the TCP/IP stack on top of the
//! esp-radio station-mode `WifiDevice` and runs `DHCPv4` once the
//! link is up.

use embassy_net::{Runner, StackResources};
use esp_radio::wifi::WifiDevice;
use static_cell::StaticCell;

/// Maximum number of concurrent sockets the firmware ever needs:
/// one for SNTP (UDP), one or two for the HTTP server (listen +
/// accepted), one for mDNS responder, plus a couple of slack.
pub const STACK_SOCKETS: usize = 6;

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
