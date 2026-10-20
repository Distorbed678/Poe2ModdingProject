//! The terrain generator's random number generator, bit-exact with the client
//! (`docs/re/terrain.md`): MSVC's `std::mt19937`, kept thread-local by the client and
//! seeded with the area seed (`0x141f9c020`), and MSVC's `uniform_int` draw (`0x141f0d7f0`).

const N: usize = 624;
const M: usize = 397;

/// `std::mt19937` (the standard 32-bit Mersenne Twister; MSVC buffers twice the state, which
/// does not change the output).
#[derive(Clone)]
pub struct Mt19937 {
    state: [u32; N],
    index: usize,
}

impl Mt19937 {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; N];
        state[0] = seed;
        for i in 1..N {
            let prev = state[i - 1];
            state[i] = (prev ^ (prev >> 30))
                .wrapping_mul(0x6c07_8965)
                .wrapping_add(i as u32);
        }
        Self { state, index: N }
    }

    fn twist(&mut self) {
        for i in 0..N {
            let y = (self.state[i] & 0x8000_0000) | (self.state[(i + 1) % N] & 0x7fff_ffff);
            let mut v = self.state[(i + M) % N] ^ (y >> 1);
            if y & 1 != 0 {
                v ^= 0x9908_b0df;
            }
            self.state[i] = v;
        }
        self.index = 0;
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.index >= N {
            self.twist();
        }
        let mut y = self.state[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }
}

/// The client's terrain RNG: the generator draws through [`TerrainRng::below`] and
/// [`TerrainRng::range`]. When `disabled` (the seed call's second argument) every draw returns
/// its lower bound.
#[derive(Clone)]
pub struct TerrainRng {
    mt: Mt19937,
    pub disabled: bool,
    /// Draws so far (raw 32-bit values), for comparing with the client's generator trace.
    pub draws: u64,
    /// When set, every bounded draw (`below`/`range`) is recorded as `(n, result)`, for
    /// lining up with the trace's `below`/`range` lines.
    pub log: Option<Vec<(u32, u32)>>,
}

impl TerrainRng {
    pub fn new(seed: u32) -> Self {
        Self {
            mt: Mt19937::new(seed),
            disabled: false,
            draws: 0,
            log: None,
        }
    }

    fn raw(&mut self) -> u32 {
        self.draws += 1;
        self.mt.next_u32()
    }

    /// MSVC `uniform_int` over `0..n` (`0x141f0d7f0`): rejection sampling on whole 32-bit
    /// draws; `n == 1` draws nothing.
    pub fn uniform(&mut self, n: u32) -> u32 {
        if n <= 1 {
            return 0;
        }
        let max = u32::MAX;
        loop {
            let v = self.raw();
            if !(max / n <= v / n && max % n != n - 1) {
                return v % n;
            }
        }
    }

    /// `0..n` (`0x141f9c270`; 0 counts as 1).
    pub fn below(&mut self, n: u32) -> u32 {
        let v = if self.disabled {
            0
        } else {
            self.uniform(n.max(1))
        };
        if let Some(log) = &mut self.log {
            log.push((n, v));
        }
        v
    }

    /// `a..=b` (`0x141f9c340`; `b < a` counts as `b = a`).
    pub fn range(&mut self, a: u32, b: u32) -> u32 {
        let v = if self.disabled {
            a
        } else {
            let b = b.max(a);
            let span = b.wrapping_sub(a);
            if span == u32::MAX {
                self.raw()
            } else {
                a.wrapping_add(self.uniform(span + 1))
            }
        };
        if let Some(log) = &mut self.log {
            log.push((b.wrapping_sub(a).wrapping_add(1), v));
        }
        v
    }
}

/// The topology choice's index (`0x141025cf0`): a C `rand`-style LCG seeded with `seed`
/// (`x = x·1103515245 + 12345`, 15 bits `(x >> 16) & 0x7fff` per step), enough bits for
/// `n - 1`, values `>= n` rejected; `n < 2` gives 0 without stepping.
pub fn lcg_index(seed: u32, n: u32) -> u32 {
    if n < 2 {
        return 0;
    }
    let bits = 32 - (n - 1).leading_zeros();
    let mut x = seed;
    loop {
        let (mut v, mut got) = (0u32, 0u32);
        while got < bits {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let take = (bits - got).min(15);
            v |= (((x >> 16) & 0x7fff) & ((1 << take) - 1)) << got;
            got += take;
        }
        if v < n {
            return v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Raw draws traced in the client after `TerrainRng_Seed(1)` (Clearfell Encampment).
    #[test]
    fn mt_matches_client_trace() {
        let mut mt = Mt19937::new(1);
        let expect = [
            1791095845, 4282876139, 3093770124, 4005303368, 491263, 550290313, 1298508491,
            4290846341, 630311759, 1013994432, 396591248, 1703301249, 799981516, 1666063943,
        ];
        for e in expect {
            assert_eq!(mt.next_u32(), e);
        }
    }

    /// The client's first bounded draws after seed 1: range over the whole u32 span, then
    /// n = 8, then n = 2 (results traced: 1791095845, 3, 0).
    #[test]
    fn uniform_matches_client_trace() {
        let mut rng = TerrainRng::new(1);
        assert_eq!(rng.range(0, u32::MAX - 1), 1791095845);
        assert_eq!(rng.uniform(8), 3);
        assert_eq!(rng.uniform(2), 0);
        assert_eq!(rng.uniform(1), 0);
        assert_eq!(rng.draws, 3);
    }

    /// Riverbank (`G1_1`, 5 topologies), seed 1000: sub-seed 2807145907 picks `intro_3`.
    #[test]
    fn topology_index() {
        assert_eq!(lcg_index(2_807_145_907, 5), 2);
    }
}
