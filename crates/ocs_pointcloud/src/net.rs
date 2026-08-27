//! HTTP agents for remote point-cloud sources.
//!
//! Name resolution orders IPv4 addresses ahead of IPv6 ones: ureq's TCP
//! connector only falls through to the next resolved address on
//! `ConnectionRefused` and timeouts, so a machine whose IPv6 path fails with
//! any other error (VPN and WFP filters commonly surface `WSAEACCES`) fails
//! every request even though the same host answers on IPv4. Trying IPv4
//! first sidesteps that; IPv6 remains available after the IPv4 candidates.
//! Mirrors the agent in the app's `src/network.rs`.

use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

/// Agent with default configuration (environment proxy pickup included).
pub(crate) fn default_agent() -> ureq::Agent {
    agent(ureq::config::Config::default())
}

/// Agent with the caller's configuration and the IPv4-first resolver.
pub(crate) fn agent(config: ureq::config::Config) -> ureq::Agent {
    ureq::Agent::with_parts(config, DefaultConnector::default(), Ipv4FirstResolver)
}

/// Resolver that orders IPv4 addresses ahead of IPv6 ones. Both families stay
/// available — IPv6 is still tried after every IPv4 candidate fails.
#[derive(Debug)]
struct Ipv4FirstResolver;

impl Resolver for Ipv4FirstResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        config: &ureq::config::Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let mut addrs = DefaultResolver::default().resolve(uri, config, timeout)?;
        ipv4_first(&mut addrs);
        Ok(addrs)
    }
}

/// Stable-reorder `addrs` so every IPv4 address precedes every IPv6 address.
fn ipv4_first(addrs: &mut ResolvedSocketAddrs) {
    let (v4, v6): (Vec<_>, Vec<_>) = addrs.iter().copied().partition(|addr| addr.is_ipv4());
    // Refilling the same count that was just drained, so push stays in range.
    addrs.truncate(0);
    for addr in v4.into_iter().chain(v6) {
        addrs.push(addr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_first_orders_v4_ahead_of_v6_and_keeps_all_addrs() {
        let mut addrs = ResolvedSocketAddrs::from_fn(|_| "0.0.0.0:0".parse().unwrap());
        let v6 = |n: u8| format!("[2001:db8::{n}]:443").parse().unwrap();
        let v4 = |n: u8| format!("10.0.0.{n}:443").parse().unwrap();
        addrs.truncate(0);
        for addr in [v6(1), v4(1), v6(2), v4(2), v6(3)] {
            addrs.push(addr);
        }

        ipv4_first(&mut addrs);

        let families: Vec<&str> =
            addrs.iter().map(|a| if a.is_ipv4() { "4" } else { "6" }).collect();
        assert_eq!(families, vec!["4", "4", "6", "6", "6"]);
        assert_eq!(addrs[0].to_string(), "10.0.0.1:443");
        assert_eq!(addrs[4].to_string(), "[2001:db8::3]:443");
    }
}
