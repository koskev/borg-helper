use std::process::Child;

use log::{debug, error, info};
use netstat2::{get_sockets_info, AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState};

use crate::utils::cmd::run_cmd_background;

fn is_port_listening(port: u16) -> bool {
    let af_flags = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
    let proto_flags = ProtocolFlags::TCP;
    let sockets_info = get_sockets_info(af_flags, proto_flags);
    debug!("Checking if port {port} is listening");
    match sockets_info {
        Ok(sockets_info) => {
            let sockets = sockets_info.iter().find(|s| match &s.protocol_socket_info {
                ProtocolSocketInfo::Tcp(tcp) => {
                    tcp.state == TcpState::Listen && tcp.local_port == port
                }
                _ => false,
            });
            sockets.is_some()
        }
        Err(_) => false,
    }
}

pub fn start_k8s_proxy(
    namespace: &str,
    name: &str,
    k8s_port: u16,
    local_port: u16,
) -> Option<Child> {
    info!("Starting proxy...");
    let cmd = format!(
        "kubectl -n {} port-forward {} {}:{}",
        namespace, name, k8s_port, local_port
    );
    let child = run_cmd_background(&cmd);
    match child {
        Ok(mut child) => {
            // Wait for proxy to run
            while !is_port_listening(local_port) {
                // Check if child returned or threw an error. If not -> Program is
                // still running and we can wait for the port
                let child_ret = child.try_wait();
                match child_ret {
                    Ok(ret) => {
                        if ret.is_some() {
                            // Process got killed while we waited for the port to be open
                            return None;
                        }
                    }
                    Err(_) => return None,
                }
            }
            Some(child)
        }
        Err(e) => {
            error!("Failed to call kubectl with error: {}", e);
            None
        }
    }
}
