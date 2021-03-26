// Copyright (c) 2019-2021, MASQ (https://masq.ai) and/or its affiliates. All rights reserved.

use crate::comm_layer::igdp::IgdpTransactor;
use crate::comm_layer::pcp::PcpTransactor;
use crate::comm_layer::pmp::PmpTransactor;
use crate::comm_layer::{AutomapError, Transactor};
use crate::probe_researcher::FirstSectionData;
use log::{info, warn};
use masq_lib::short_writeln;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::str::FromStr;
use std::time::{Duration, Instant};

pub fn test_pcp(test_port: u16, _port_is_manual: bool) -> Result<(IpAddr, u16, Box<dyn Transactor>, bool), String> {
    let transactor = PcpTransactor::default();
    let (router_ip, status) = find_router(TestStatus::new(), &transactor, " PCP ");
    let (status, port) = test_common(test_port, status, router_ip, &transactor);
    if !status.cumulative_success {
        Err(String::from(
            "\
Either PCP is not implemented on your router or we're not doing it right\n\
------------------------------------------------------------------------\n",
        ))
    } else {
        Ok((router_ip, port, Box::new(transactor), false))
    }
}

pub fn test_pmp(test_port: u16, _port_is_manual: bool) -> Result<(IpAddr, u16, Box<dyn Transactor>, bool), String> {
    let transactor = PmpTransactor::default();
    let (router_ip, status) = find_router(TestStatus::new(), &transactor, " PMP ");
    let (status, port) = test_common(test_port, status, router_ip, &transactor);
    if !status.cumulative_success {
        Err(String::from(
            "\
Either PMP is not implemented on your router or we're not doing it right\n\
------------------------------------------------------------------------\n",
        ))
    } else {
        Ok((router_ip, port, Box::new(transactor), false))
    }
}

pub fn test_igdp(
    test_port: u16,
    _port_is_manual: bool,
) -> Result<(IpAddr, u16, Box<dyn Transactor>, bool), String> {
    let transactor = IgdpTransactor::default();
    let (router_ip, status) = find_router(TestStatus::new(), &transactor, " IGDP ");
    let status = seek_public_ip(status, router_ip, &transactor);
    let (mut port, mut status) = poke_firewall_hole(test_port, status, router_ip, &transactor);
    let mut permanent_hole = false;
    let status = if status.step_success {
        status
    } else if status
        .step_error
        .as_ref()
        .expect("Step failure, but no error recorded!")
        == &AutomapError::AddMappingError("OnlyPermanentLeasesSupported".to_string())
    {
        let warning = "IGDP detected but this router doesn't like keeping track of holes and closing them on a schedule. We'll try a permanent one.";
        warn!("{}", warning);
        status.cumulative_success = true; // adjustment for retry
        let (port_permanent, status) = poke_permanent_firewall_hole(test_port, status, router_ip, &transactor);
        port = port_permanent;
        permanent_hole = true;
        status
    } else {
        status
    };
    if !status.cumulative_success {
        Err(String::from(
            "\
Either IGDP is not implemented on your router or we're not doing it right\n\
-------------------------------------------------------------------------\n",
        ))
    } else {
        Ok((router_ip, port, Box::new(transactor), permanent_hole))
    }
}

fn test_common(
    test_port: u16,
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> (TestStatus, u16) {
    let status = seek_public_ip(status, router_ip, transactor);
    let (port, status) = poke_firewall_hole(test_port, status, router_ip, transactor);
    (status, port)
}

fn find_router(
    status: TestStatus,
    transactor: &dyn Transactor,
    tested_protocol: &str,
) -> (IpAddr, TestStatus) {
    info!("=============={}===============", tested_protocol);
    info!("{}. Looking for routers on the subnet...", status.step);
    let timer = Timer::new();
    match transactor.find_routers() {
        Ok(list) => {
            let found_router_ip = list[0];
            info!(
                "...found a router after {} at {}.",
                timer.ms(),
                found_router_ip
            );
            (found_router_ip, status.succeed())
        }
        Err(e) => {
            info!("...failed after {}: {:?}", timer.ms(), e);
            (IpAddr::from_str("0.0.0.0").unwrap(), status.fail(e))
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
    info!("{}. Seeking public IP address...", status.step);
    let timer = Timer::new();
    match transactor.get_public_ip(router_ip) {
        Ok(public_ip) => {
            info! ("...found after {}: {}  Is that correct? (Maybe don't publish this without redacting it?)", timer.ms(), public_ip);
            status.succeed()
        }
        Err(e) => {
            info!("...failed after {}: {:?}", timer.ms(), e);
            status.fail(e)
        }
    }
}

fn poke_firewall_hole(
    test_port: u16,
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> (u16, TestStatus) {
    if status.fatal {
        return (0, status);
    }
    {
        let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), test_port);
        let _socket =
            match UdpSocket::bind(socket_addr) {
                Ok(s) => s,
                Err(e) => {
                    info!("Failed to open local port {}; giving up. ({:?})", test_port, e);
                    return (
                        test_port,
                        status.abort(AutomapError::SocketBindingError(
                            format!("{:?}", e),
                            socket_addr,
                        )),
                    );
                }
            };
    }
    info!(
        "{}. Poking a 3-second hole in the firewall for port {}...",
        status.step, test_port
    );
    let timer = Timer::new();
    match transactor.add_mapping(router_ip, test_port, 5) {
        Ok(delay) => {
            info!(
                "...success after {}! Recommended remap delay is {} seconds.",
                timer.ms(),
                delay
            );
            (test_port, status.succeed().permanent_only(false))
        }
        Err(e) if e == AutomapError::PermanentLeasesOnly => {
            let warning = format!(
                "{} detected but this router doesn't like keeping track of holes and closing them on a schedule. We'll try a permanent one.",
                transactor.method()
            );
            println!("{}", warning);
            warn!("{}", warning);
            let mut out_status = status.clone();
            out_status.step += 1;
            out_status.cumulative_success = true; // adjustment for retry
            poke_permanent_firewall_hole(test_port, out_status, router_ip, transactor)
        }
        Err(e) => {
            info!("...failed after {}: {:?}", timer.ms(), e);
            (test_port, status.fail(e))
        }
    }
}

fn poke_permanent_firewall_hole(
    test_port: u16,
    status: TestStatus,
    router_ip: IpAddr,
    transactor: &dyn Transactor,
) -> (u16, TestStatus) {
    if status.fatal {
        return (0, status);
    }
    info!(
        "{}. Poking a permanent hole in the firewall for port {}...",
        status.step, test_port
    );
    let timer = Timer::new();
    match transactor.add_mapping(router_ip, test_port, 0) {
        Ok(delay) => {
            info!(
                "...success after {}! Recommended remap delay is {} seconds.",
                timer.ms(),
                delay
            );
            (test_port, status.succeed().permanent_only(true))
        }
        Err(e) => {
            info!("...failed after {}: {:?}", timer.ms(), e);
            (test_port, status.fail(e))
        }
    }
}

#[allow(clippy::result_unit_err)]
pub fn remove_firewall_hole(
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    params: FirstSectionData,
) -> Result<(), ()> {
    info!("Removing the port-{} hole in the firewall...", params.port);
    let permanent_only = params.permanent_only.expect("permanent_only should be set by now");
    let timer = Timer::new();
    match params.transactor.delete_mapping(params.ip, params.port) {
        Ok(_) => {
            if permanent_only {
                info!("...success after {}, but this protocol only works with permanent ports on this router. Argh.", timer.ms());
            }
            else {
                info!("...success after {}!", timer.ms());
            }
            short_writeln!(stdout, "Port was closed successfully");
            stdout.flush().expect("flush failed");
            Ok(())
        }
        Err(e) => {
            if permanent_only {
                warn!("...failed after {}: {:?}", timer.ms(), e);
                let warning =  format!("You'll need to close  port {} yourself in your router's administration pages. \
            .\nYou may also look into the log. \nSorry...I didn't do it on purpose...",params.port);
                warn!("{}", warning);
                short_writeln!(stderr, "{}", warning);
                stderr.flush().expect("flush failed");
            }
            else {
                info!("...failed after {}: {:?} (Note: the hole will disappear on its own in a few seconds.)", timer.ms(), e);
                short_writeln!(stderr,"Operation failed, but don't worry, the hole will disappear on its own in a few seconds.");
                stderr.flush().expect("flush failed");
            }
            Err(())
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

#[derive (Clone)]
struct TestStatus {
    step: usize,
    step_success: bool,
    step_error: Option<AutomapError>,
    cumulative_success: bool,
    fatal: bool,
    permanent_only: Option<bool>,
}

impl TestStatus {
    fn new() -> Self {
        Self {
            step: 1,
            step_success: true,
            step_error: None,
            cumulative_success: true,
            fatal: false,
            permanent_only: None,
        }
    }

    fn succeed(self) -> Self {
        Self {
            step: self.step + 1,
            step_success: true,
            step_error: None,
            cumulative_success: self.cumulative_success,
            fatal: false,
            permanent_only: self.permanent_only,
        }
    }

    fn fail(self, error: AutomapError) -> Self {
        Self {
            step: self.step + 1,
            step_success: false,
            step_error: Some(error),
            cumulative_success: false,
            fatal: false,
            permanent_only: self.permanent_only,
        }
    }

    fn abort(self, error: AutomapError) -> Self {
        Self {
            step: self.step + 1,
            step_success: false,
            step_error: Some(error),
            cumulative_success: false,
            fatal: true,
            permanent_only: self.permanent_only,
        }
    }

    fn permanent_only(self, permanent_only: bool) -> Self {
        Self {
            step: self.step,
            step_success: self.step_success,
            step_error: self.step_error,
            cumulative_success: self.cumulative_success,
            fatal: self.fatal,
            permanent_only: Some(permanent_only),
        }
    }
}
