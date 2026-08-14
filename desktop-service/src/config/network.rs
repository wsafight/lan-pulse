use std::{
    net::{IpAddr, Ipv4Addr, UdpSocket},
    process::Command,
};

pub fn detect_lan_ip() -> Option<String> {
    let hostname_ips = detect_lan_ips_from_hostname();
    best_lan_ip(&hostname_ips)
        .or_else(detect_lan_ip_from_route)
        .map(|ip| ip.to_string())
}

fn detect_lan_ip_from_route() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;

    match addr.ip() {
        IpAddr::V4(ip) if is_candidate_lan_ip(ip) => Some(ip),
        _ => None,
    }
}

fn detect_lan_ips_from_hostname() -> Vec<Ipv4Addr> {
    let output = match Command::new("hostname").arg("-I").output() {
        Ok(output) => output,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    let stdout = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout,
        Err(_) => return Vec::new(),
    };

    parse_hostname_ips(&stdout)
}

fn parse_hostname_ips(stdout: &str) -> Vec<Ipv4Addr> {
    stdout
        .split_whitespace()
        .filter_map(|part| part.parse::<IpAddr>().ok())
        .filter_map(|ip| match ip {
            IpAddr::V4(ip) if is_candidate_lan_ip(ip) => Some(ip),
            _ => None,
        })
        .collect()
}

fn best_lan_ip(ips: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    ips.iter().copied().min_by_key(|ip| ipv4_score(*ip))
}

fn is_candidate_lan_ip(ip: Ipv4Addr) -> bool {
    !ip.is_loopback() && !ip.is_unspecified()
}

fn ipv4_score(ip: Ipv4Addr) -> u8 {
    let octets = ip.octets();

    if octets[0] == 192 && octets[1] == 168 {
        return 0;
    }

    if octets[0] == 10 {
        return 1;
    }

    if octets[0] == 172 && (16..=31).contains(&octets[1]) {
        return 2;
    }

    if octets[0] == 169 && octets[1] == 254 {
        return 4;
    }

    3
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::{best_lan_ip, ipv4_score, is_candidate_lan_ip, parse_hostname_ips};

    #[test]
    fn prefers_common_lan_before_proxy_or_docker_ips() {
        assert!(
            ipv4_score(Ipv4Addr::new(192, 168, 31, 59)) < ipv4_score(Ipv4Addr::new(198, 18, 0, 1))
        );
        assert!(
            ipv4_score(Ipv4Addr::new(192, 168, 31, 59)) < ipv4_score(Ipv4Addr::new(172, 17, 0, 1))
        );
    }

    #[test]
    fn chooses_best_hostname_candidate() {
        let ips = [
            Ipv4Addr::new(169, 254, 1, 9),
            Ipv4Addr::new(172, 20, 0, 3),
            Ipv4Addr::new(10, 1, 2, 3),
            Ipv4Addr::new(192, 168, 1, 9),
        ];

        assert_eq!(best_lan_ip(&ips), Some(Ipv4Addr::new(192, 168, 1, 9)));
    }

    #[test]
    fn best_lan_ip_returns_none_for_empty_input() {
        assert_eq!(best_lan_ip(&[]), None);
    }

    #[test]
    fn excludes_loopback_and_unspecified_addresses() {
        assert!(!is_candidate_lan_ip(Ipv4Addr::LOCALHOST));
        assert!(!is_candidate_lan_ip(Ipv4Addr::UNSPECIFIED));
        assert!(is_candidate_lan_ip(Ipv4Addr::new(192, 168, 1, 9)));
    }

    #[test]
    fn parses_hostname_output_keeping_only_candidate_ipv4_addresses() {
        let ips = parse_hostname_ips("127.0.0.1 0.0.0.0 192.168.1.9 fe80::1 10.0.0.2 bad");

        assert_eq!(
            ips,
            vec![Ipv4Addr::new(192, 168, 1, 9), Ipv4Addr::new(10, 0, 0, 2)]
        );
    }

    #[test]
    fn scores_lan_ranges_before_public_and_link_local_addresses() {
        assert_eq!(ipv4_score(Ipv4Addr::new(192, 168, 1, 9)), 0);
        assert_eq!(ipv4_score(Ipv4Addr::new(10, 0, 0, 2)), 1);
        assert_eq!(ipv4_score(Ipv4Addr::new(172, 16, 0, 2)), 2);
        assert_eq!(ipv4_score(Ipv4Addr::new(172, 31, 0, 2)), 2);
        assert_eq!(ipv4_score(Ipv4Addr::new(172, 15, 0, 2)), 3);
        assert_eq!(ipv4_score(Ipv4Addr::new(172, 32, 0, 2)), 3);
        assert_eq!(ipv4_score(Ipv4Addr::new(8, 8, 8, 8)), 3);
        assert_eq!(ipv4_score(Ipv4Addr::new(169, 254, 1, 1)), 4);
    }
}
