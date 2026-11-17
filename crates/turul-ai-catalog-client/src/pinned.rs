//! Connect-to-vetted: the resolver hands the connector exactly the vetted addresses.

use std::net::SocketAddr;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

pub(crate) struct PinnedResolver {
    host: String,
    addrs: Vec<SocketAddr>,
}

impl PinnedResolver {
    pub(crate) fn new(host: &str, addrs: Vec<SocketAddr>) -> Self {
        PinnedResolver {
            host: host.to_ascii_lowercase(),
            addrs,
        }
    }
}

impl Resolve for PinnedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let result: Result<Addrs, Box<dyn std::error::Error + Send + Sync>> =
            if name.as_str().eq_ignore_ascii_case(&self.host) {
                Ok(Box::new(self.addrs.clone().into_iter()))
            } else {
                Err(format!("name {} is not the vetted host", name.as_str()).into())
            };
        Box::pin(std::future::ready(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll, Waker};

    fn run(r: &PinnedResolver, name: &str) -> Result<Vec<SocketAddr>, String> {
        let mut fut = r.resolve(name.parse::<Name>().unwrap());
        let mut cx = Context::from_waker(Waker::noop());
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(a)) => Ok(a.collect()),
            Poll::Ready(Err(e)) => Err(e.to_string()),
            Poll::Pending => panic!("pinned resolver must be ready"),
        }
    }

    #[test]
    fn resolver_returns_exactly_the_vetted_addresses_for_the_hop_host_only() {
        let addrs: Vec<SocketAddr> = vec!["93.184.216.34:443".parse().unwrap()];
        let r = PinnedResolver::new("Example.com", addrs.clone());
        assert_eq!(run(&r, "example.com").unwrap(), addrs);
        assert!(run(&r, "other.example").is_err());
    }
}
