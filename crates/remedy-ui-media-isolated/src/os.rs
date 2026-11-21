//! The operating system: starting the helper sandboxed, the channel to it, shared memory,
//! and the sandbox the helper shuts itself into. All of the crate's unsafe code is here, each
//! platform in its own module behind the same safe types:
//!
//! - `Shm`: a shared memory region the app makes for one request (its size is the request's)
//! - `Child`: a running helper, seen from the app; `Killer` stops it from another thread
//! - `Parent`: the app, seen from the helper; `Mapped`: a region the helper was handed
//! - `lock_down`: what the helper does to itself before it reads any request

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
mod unix;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) use unix::*;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
pub(crate) use frame::*;

/// Why there is no helper on this platform.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos", target_os = "android")))]
pub(crate) fn unsupported_reason() -> String {
    if cfg!(target_os = "ios") {
        "iOS does not let apps start helper processes".into()
    } else {
        "this platform has no helper sandbox here (RemedyOS isolation domains come with its \
         bridge)"
            .into()
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", target_os = "android"))]
mod frame {
    use std::io;

    use crate::Layer;

    /// Most regions one frame carries (a request's input and output).
    pub(crate) const MAX_REGIONS: usize = 2;
    const REGION: usize = 17;

    /// A shared region as a frame describes it: the handle value the helper uses (Windows;
    /// file descriptors travel beside the frame on Unix), its length, and whether the
    /// helper may write it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct Region {
        pub handle: u64,
        pub len: u64,
        pub writable: bool,
    }

    /// The bytes before a frame's payload: its length, the region count, the regions.
    pub(crate) fn header(payload: usize, regions: &[Region]) -> io::Result<Vec<u8>> {
        let len = u16::try_from(payload).map_err(|_| io::Error::other("message too long"))?;
        if regions.len() > MAX_REGIONS {
            return Err(io::Error::other("too many regions"));
        }
        let mut out = Vec::with_capacity(3 + regions.len() * REGION);
        out.extend_from_slice(&len.to_le_bytes());
        out.push(regions.len() as u8);
        for r in regions {
            out.extend_from_slice(&r.handle.to_le_bytes());
            out.extend_from_slice(&r.len.to_le_bytes());
            out.push(u8::from(r.writable));
        }
        Ok(out)
    }

    /// The payload length and region count from a frame's first three bytes.
    pub(crate) fn lead(b: [u8; 3]) -> io::Result<(usize, usize)> {
        let [l0, l1, n] = b;
        let n = usize::from(n);
        if n > MAX_REGIONS {
            return Err(io::Error::other("too many regions"));
        }
        Ok((usize::from(u16::from_le_bytes([l0, l1])), n))
    }

    /// The regions following the lead.
    pub(crate) fn regions(b: &[u8]) -> io::Result<Vec<Region>> {
        let bad = || io::Error::other("bad region");
        b.chunks(REGION)
            .map(|c| {
                let (h, rest) = c.split_first_chunk::<8>().ok_or_else(bad)?;
                let (l, rest) = rest.split_first_chunk::<8>().ok_or_else(bad)?;
                let w = match rest {
                    [0] => false,
                    [1] => true,
                    _ => return Err(bad()),
                };
                Ok(Region { handle: u64::from_le_bytes(*h), len: u64::from_le_bytes(*l), writable: w })
            })
            .collect()
    }

    /// Bytes of region descriptions for `n` regions.
    pub(crate) fn regions_len(n: usize) -> usize {
        n * REGION
    }

    /// The layers as the bit set a helper reports.
    pub(crate) fn layer_bits(layers: &[Layer]) -> u32 {
        layers.iter().fold(0, |bits, l| {
            bits | match *l {
                Layer::AppContainer => 1,
                Layer::RestrictedToken => 1 << 1,
                Layer::JobObject => 1 << 2,
                Layer::Seccomp => 1 << 3,
                Layer::Landlock { abi } => (1 << 4) | ((abi.min(255)) << 16),
                Layer::NetworkNamespace => 1 << 5,
                Layer::ResourceLimits => 1 << 6,
                Layer::AppSandbox => 1 << 7,
                Layer::IsolatedProcess => 1 << 8,
            }
        })
    }

    /// The layers a bit set names.
    pub(crate) fn layers_from_bits(bits: u32) -> Vec<Layer> {
        let mut out = Vec::new();
        let named = [
            (1, Layer::AppContainer),
            (1 << 1, Layer::RestrictedToken),
            (1 << 2, Layer::JobObject),
            (1 << 3, Layer::Seccomp),
            (1 << 4, Layer::Landlock { abi: (bits >> 16) & 0xFF }),
            (1 << 5, Layer::NetworkNamespace),
            (1 << 6, Layer::ResourceLimits),
            (1 << 7, Layer::AppSandbox),
            (1 << 8, Layer::IsolatedProcess),
        ];
        for (bit, layer) in named {
            if bits & bit != 0 {
                out.push(layer);
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn frames_and_layers_round_trip() {
            let rs = [
                Region { handle: 0x1234, len: 99, writable: true },
                Region { handle: 0, len: 1, writable: false },
            ];
            let h = header(5, &rs).unwrap_or_default();
            let (len, n) = lead(h.first_chunk::<3>().copied().unwrap_or_default()).unwrap_or((0, 0));
            assert_eq!((len, n), (5, 2));
            assert_eq!(regions(h.get(3..).unwrap_or_default()).ok().as_deref(), Some(&rs[..]));
            assert!(lead([0, 0, 3]).is_err());
            assert!(header(70_000, &[]).is_err());
            let layers = vec![Layer::Seccomp, Layer::Landlock { abi: 6 }, Layer::ResourceLimits];
            assert_eq!(layers_from_bits(layer_bits(&layers)), layers);
        }
    }
}
