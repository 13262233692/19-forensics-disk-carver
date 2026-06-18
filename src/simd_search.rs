#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
use std::arch::x86_64::*;

pub struct SimdSearcher {
    pattern: Vec<u8>,
    first_byte: u8,
    pattern_len: usize,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    use_avx512: bool,
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    use_avx2: bool,
}

impl SimdSearcher {
    pub fn new(pattern: &[u8]) -> Self {
        let (avx512, avx2) = detect_cpu_features();
        SimdSearcher {
            pattern: pattern.to_vec(),
            first_byte: pattern[0],
            pattern_len: pattern.len(),
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            use_avx512: avx512,
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            use_avx2: avx2,
        }
    }

    pub fn find_all(&self, haystack: &[u8]) -> Vec<usize> {
        if self.pattern_len > haystack.len() {
            return Vec::new();
        }

        if self.pattern_len == 1 {
            return find_all_bytes(haystack, self.first_byte);
        }

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        {
            if self.use_avx512 && self.pattern_len <= 64 {
                return unsafe { self.find_all_avx512(haystack) };
            }
            if self.use_avx2 && self.pattern_len <= 32 {
                return unsafe { self.find_all_avx2(haystack) };
            }
        }

        self.find_all_fallback(haystack)
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[target_feature(enable = "avx512f,avx512bw")]
    unsafe fn find_all_avx512(&self, haystack: &[u8]) -> Vec<usize> {
        let mut results = Vec::new();
        let n = haystack.len();
        let p = self.pattern.as_slice();
        let plen = self.pattern_len;

        if plen == 0 || n < plen {
            return results;
        }

        let first_vec = _mm512_set1_epi8(p[0] as i8);

        let mut i = 0;
        while i + 64 <= n {
            let chunk = _mm512_loadu_si512(haystack.as_ptr().add(i) as *const _);
            let mut mask = _mm512_cmpeq_epi8_mask(first_vec, chunk);

            while mask != 0 {
                let pos = mask.trailing_zeros() as usize;
                mask &= mask - 1;

                let offset = i + pos;
                if offset + plen > n {
                    continue;
                }

                if plen > 1 {
                    let last_pos = offset + plen - 1;
                    if haystack[last_pos] != p[plen - 1] {
                        continue;
                    }
                }

                if verify_pattern(&haystack[offset..offset + plen], p) {
                    results.push(offset);
                }
            }

            i += 64;
        }

        while i < n {
            if haystack[i] == p[0] && i + plen <= n && verify_pattern(&haystack[i..i + plen], p) {
                results.push(i);
            }
            i += 1;
        }

        results
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[target_feature(enable = "avx2")]
    unsafe fn find_all_avx2(&self, haystack: &[u8]) -> Vec<usize> {
        let mut results = Vec::new();
        let n = haystack.len();
        let p = self.pattern.as_slice();
        let plen = self.pattern_len;

        if plen == 0 || n < plen {
            return results;
        }

        let first_vec = _mm256_set1_epi8(p[0] as i8);

        let mut i = 0;
        while i + 32 <= n {
            let chunk = _mm256_loadu_si256(haystack.as_ptr().add(i) as *const _);
            let cmp = _mm256_cmpeq_epi8(first_vec, chunk);
            let mut mask = _mm256_movemask_epi8(cmp) as u32;

            while mask != 0 {
                let pos = mask.trailing_zeros() as usize;
                mask &= mask - 1;

                let offset = i + pos;
                if offset + plen > n {
                    continue;
                }

                if plen > 1 {
                    let last_pos = offset + plen - 1;
                    if haystack[last_pos] != p[plen - 1] {
                        continue;
                    }
                }

                if verify_pattern(&haystack[offset..offset + plen], p) {
                    results.push(offset);
                }
            }

            i += 32;
        }

        while i < n {
            if haystack[i] == p[0] && i + plen <= n && verify_pattern(&haystack[i..i + plen], p) {
                results.push(i);
            }
            i += 1;
        }

        results
    }

    fn find_all_fallback(&self, haystack: &[u8]) -> Vec<usize> {
        let mut results = Vec::new();
        let n = haystack.len();
        let p = self.pattern.as_slice();
        let plen = self.pattern_len;

        if plen == 0 || n < plen {
            return results;
        }

        let mut i = 0;
        while i + plen <= n {
            if haystack[i] == p[0] && verify_pattern(&haystack[i..i + plen], p) {
                results.push(i);
            }
            i += 1;
        }

        results
    }

    pub fn find_first(&self, haystack: &[u8]) -> Option<usize> {
        self.find_all(haystack).into_iter().next()
    }
}

fn find_all_bytes(haystack: &[u8], byte: u8) -> Vec<usize> {
    let mut results = Vec::new();
    for (i, &b) in haystack.iter().enumerate() {
        if b == byte {
            results.push(i);
        }
    }
    results
}

#[inline(always)]
fn verify_pattern(window: &[u8], pattern: &[u8]) -> bool {
    if window.len() < pattern.len() {
        return false;
    }
    for i in 0..pattern.len() {
        if window[i] != pattern[i] {
            return false;
        }
    }
    true
}

pub struct BatchSimdSearcher {
    searchers: Vec<(usize, SimdSearcher)>,
}

impl BatchSimdSearcher {
    pub fn new(patterns: &[&[u8]]) -> Self {
        let searchers = patterns
            .iter()
            .enumerate()
            .map(|(i, p)| (i, SimdSearcher::new(p)))
            .collect();
        BatchSimdSearcher { searchers }
    }

    pub fn find_all(&self, haystack: &[u8]) -> Vec<(usize, usize)> {
        let mut results = Vec::new();

        for (sig_idx, searcher) in &self.searchers {
            for pos in searcher.find_all(haystack) {
                results.push((*sig_idx, pos));
            }
        }

        results.sort_by_key(|&(_, pos)| pos);
        results
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_cpu_features() -> (bool, bool) {
    let avx512 = is_x86_feature_detected!("avx512f")
        && is_x86_feature_detected!("avx512bw")
        && is_x86_feature_detected!("avx512vl");
    let avx2 = is_x86_feature_detected!("avx2");
    (avx512, avx2)
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn detect_cpu_features() -> (bool, bool) {
    (false, false)
}

pub fn simd_available() -> (bool, bool) {
    detect_cpu_features()
}
