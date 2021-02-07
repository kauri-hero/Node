// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use automap_lib::comm_layer::igdp::IgdpTransactor;
use automap_lib::comm_layer::pcp::PcpTransactor;
use automap_lib::comm_layer::pmp::PmpTransactor;
use automap_lib::comm_layer::{AutomapError, Transactor};
use masq_lib::utils::find_free_port;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::str::FromStr;
use std::time::{Duration, Instant};

pub fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    if args.len() != 2 {
        println!("Usage: automap <IP address of your router>");
        return;
    }
    let ip_string = args[1].as_str();
    let router_ip = match IpAddr::from_str(ip_string) {
        Ok(ip) => ip,
        Err(e) => {
            println!(
                "'{}' is not a properly-formatted IP address: {:?}",
                ip_string, e
            );
            return;
        }
    };

    test_pcp(router_ip);
    test_pmp(router_ip);
    test_igdp(router_ip);
}

fn test_pcp(router_ip: IpAddr) {
    println!("\n====== PCP TESTS ======");
    let transactor = PcpTransactor::default();
    let status = test_common(TestStatus::new(), router_ip, &transactor);
    if status.cumulative_success {
        println!(
            "====== PCP is implemented on your router and we can successfully employ it ======\n"
        )
    } else {
        println! ("====== Either PCP is not implemented on your router or we're not doing it right ======\n")
    }
}

fn test_pmp(router_ip: IpAddr) {
    println!("\n====== PMP TESTS ======");
    let transactor = PmpTransactor::default();
    let status = test_common(TestStatus::new(), router_ip, &transactor);
    if status.cumulative_success {
        println!(
            "====== PMP is implemented on your router and we can successfully employ it ======\n"
        )
    } else {
        println! ("====== Either PMP is not implemented on your router or we're not doing it right ======\n")
    }
}

fn test_igdp(router_ip: IpAddr) {
    println!("\n====== IGDP TESTS ======");
    let transactor = IgdpTransactor::default();
    let status = find_router(TestStatus::new(), router_ip, &transactor);
    let status = seek_public_ip(status, router_ip, &transactor);
    let (port, mut status) = poke_firewall_hole(status, router_ip, &transactor);
    let status = if status.step_success {
        remove_firewall_hole(port, status, router_ip, &transactor)
    } else {
        if status
            .step_error
            .as_ref()
            .expect("Step failure, but no error recorded!")
            == &AutomapError::AddMappingError("OnlyPermanentLeasesSupported".to_string())
        {
            println! ("This router doesn't like keeping track of holes and closing them on a schedule. We'll try a permanent one.");
            status.cumulative_success = true; // adjustment for retry
            let (port, status) = poke_permanent_firewall_hole(status, router_ip, &transactor);
            if status.step_success {
                remove_permanent_firewall_hole(port, status, router_ip, &transactor)
            } else {
                status
            }
        } else {
            status
        }
    };
    if status.cumulative_success {
        println!(
            "====== IGDP is implemented on your router and we can successfully employ it ======\n"
        )
    } else {
        println! ("====== Either IGDP is not implemented on your router or we're not doing it right ======\n")
    }
}

fn test_common(status: TestStatus, router_ip: IpAddr, transactor: &dyn Transactor) -> TestStatus {
    let status = seek_public_ip(status, router_ip, transactor);
    let (port, mut status) = poke_firewall_hole(status, router_ip, transactor);
    if status.step_success {
        status = remove_firewall_hole(port, status, router_ip, transactor);
    }
    status
}

fn find_router(status: TestStatus, router_ip: IpAddr, transactor: &dyn Transactor) -> TestStatus {
    println!("{}. Looking for routers on the subnet...", status.step);
    let timer = Timer::new();
    match transactor.find_routers() {
        Ok(list) => {
            let found_router_ip = list[0];
            if found_router_ip == router_ip {
                println!(
                    "...found a router after {} at {}, just like you said.",
                    timer.ms(),
                    found_router_ip
                );
            } else {
                println!(
                    "...found a router after {} at {}, but you said I'd find it at {}.",
                    timer.ms(),
                    found_router_ip,
                    router_ip
                );
            }
            status.succeed()
        }
        Err(e) => {
            println!("...failed after {}: {:?}", timer.ms(), e);
            status.fail(e)
        }
    }
}

fn seek_public_ip(
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> TestStatus {
    if status.fatal {
        return status;
    }
    println!("{}. Seeking public IP address...", status.step);
    let timer = Timer::new();
    match transactor.get_public_ip(router_ip) {
        Ok(public_ip) => {
            println! ("...found after {}: {}  Is that correct? (Maybe don't publish this without redacting it?)", timer.ms(), public_ip);
            status.succeed()
        }
        Err(e) => {
            println!("...failed after {}: {:?}", timer.ms(), e);
            status.fail(e)
        }
    }
}

fn poke_firewall_hole(
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> (u16, TestStatus) {
    if status.fatal {
        return (0, status);
    }
    let port = find_free_port();
    let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);
    let _socket =
        match UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port)) {
            Ok(s) => s,
            Err(e) => {
                println!("Failed to open local port {}; giving up. ({:?})", port, e);
                return (
                    port,
                    status.abort(AutomapError::SocketBindingError(
                        format!("{:?}", e),
                        socket_addr,
                    )),
                );
            }
        };
    println!(
        "{}. Poking a 3-second hole in the firewall for port {}...",
        status.step, port
    );
    let timer = Timer::new();
    match transactor.add_mapping(router_ip, port, 5) {
        Ok(delay) => {
            println!(
                "...success after {}! Recommended remap delay is {} seconds.",
                timer.ms(),
                delay
            );
            (port, status.succeed())
        }
        Err(e) => {
            println!("...failed after {}: {:?}", timer.ms(), e);
            (port, status.fail(e))
        }
    }
}

fn poke_permanent_firewall_hole(
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> (u16, TestStatus) {
    if status.fatal {
        return (0, status);
    }
    let port = find_free_port();
    let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port);
    let _socket =
        match UdpSocket::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), port)) {
            Ok(s) => s,
            Err(e) => {
                println!("Failed to open local port {}; giving up. ({:?})", port, e);
                return (
                    port,
                    status.abort(AutomapError::SocketBindingError(
                        format!("{:?}", e),
                        socket_addr,
                    )),
                );
            }
        };
    println!(
        "{}. Poking a permanent hole in the firewall for port {}...",
        status.step, port
    );
    let timer = Timer::new();
    match transactor.add_mapping(router_ip, port, 0) {
        Ok(delay) => {
            println!(
                "...success after {}! Recommended remap delay is {} seconds.",
                timer.ms(),
                delay
            );
            (port, status.succeed())
        }
        Err(e) => {
            println!("...failed after {}: {:?}", timer.ms(), e);
            (port, status.fail(e))
        }
    }
}

fn remove_firewall_hole(
    port: u16,
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> TestStatus {
    if status.fatal {
        return status;
    }
    println!(
        "{}. Removing the port-{} hole in the firewall...",
        status.step, port
    );
    let timer = Timer::new();
    match transactor.delete_mapping(router_ip, port) {
        Ok(_) => {
            println!("...success after {}!", timer.ms());
            status.succeed()
        }
        Err(e) => {
            println! ("...failed after {}: {:?} (Note: the hole will disappear on its own in a few seconds.)", timer.ms(), e);
            status.fail(e)
        }
    }
}

fn remove_permanent_firewall_hole(
    port: u16,
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> TestStatus {
    if status.fatal {
        return status;
    }
    println!(
        "{}. Removing the port-{} hole in the firewall...",
        status.step, port
    );
    let timer = Timer::new();
    match transactor.delete_mapping(router_ip, port) {
        Ok(_) => {
            println! ("...success after {}, but IGDP only works with permanent ports on this router. Argh.", timer.ms());
            status.succeed()
        }
        Err(e) => {
            println!("...failed after {}: {:?}", timer.ms(), e);
            println!("This is a problem! You have a permanent hole in your firewall that I can't");
            println!(
                "close. You'll need to close it yourself in your router's administration pages."
            );
            println!("Sorry...I didn't do it on purpose...");
            status.fail(e)
        }
    }
}

struct Timer {
    began_at: Instant,
}

impl Timer {
    pub fn new() -> Self {
        Self {
            began_at: Instant::now(),
        }
    }

    pub fn stop(self) -> Duration {
        let ended_at = Instant::now();
        ended_at.duration_since(self.began_at)
    }

    pub fn ms(self) -> String {
        let interval = self.stop();
        format!("{}ms", interval.as_millis())
    }
}

struct TestStatus {
    step: usize,
    step_success: bool,
    step_error: Option<AutomapError>,
    cumulative_success: bool,
    fatal: bool,
}

impl TestStatus {
    fn new() -> Self {
        Self {
            step: 1,
            step_success: true,
            step_error: None,
            cumulative_success: true,
            fatal: false,
        }
    }

    fn succeed(self) -> Self {
        Self {
            step: self.step + 1,
            step_success: true,
            step_error: None,
            cumulative_success: self.cumulative_success,
            fatal: false,
        }
    }

    fn fail(self, error: AutomapError) -> Self {
        Self {
            step: self.step + 1,
            step_success: false,
            step_error: Some(error),
            cumulative_success: false,
            fatal: false,
        }
    }

    fn abort(self, error: AutomapError) -> Self {
        Self {
            step: self.step + 1,
            step_success: false,
            step_error: Some(error),
            cumulative_success: false,
            fatal: true,
        }
    }
}