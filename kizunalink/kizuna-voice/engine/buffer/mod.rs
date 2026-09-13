// Copyright (c) 2026 nikcodex (KizunaLink)
// Licensed under the MIT License

pub mod pool;
pub mod ring;

pub use pool::{BufferPool, get_byte_pool};
pub use ring::RingBuffer;

/// A specialized vector for audio data that can be efficiently pooled.
pub type PooledBuffer = Vec<i16>;

/// Converts native-endian PCM samples to bytes.
pub fn cast_to_bytes(v: PooledBuffer) -> Vec<u8> {
    v.into_iter().flat_map(i16::to_ne_bytes).collect()
}

/// Converts complete native-endian byte pairs to PCM samples.
pub fn cast_from_bytes(v: Vec<u8>) -> PooledBuffer {
    v.as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| i16::from_ne_bytes(*bytes))
        .collect()
}

/// Returns a byte-slice view of the pooled buffer.
#[inline]
pub fn as_byte_slice(v: &[i16]) -> &[u8] {
    // SAFETY: `u8` has alignment 1 and every byte pattern is valid. The output length
    // exactly covers the initialized `i16` slice and cannot outlive it.
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, v.len() * 2) }
}

/// Iterates over complete native-endian PCM samples in a byte slice.
#[inline]
pub fn i16_samples(v: &[u8]) -> impl Iterator<Item = i16> + '_ {
    v.as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| i16::from_ne_bytes(*bytes))
}

/// Returns an `i16` view when the input has the required alignment and an even length.
///
/// # Panics
///
/// Panics when `v` does not cover a complete, properly aligned sequence of `i16` values.
#[inline]
pub fn as_i16_slice(v: &[u8]) -> &[i16] {
    // SAFETY: `align_to` only partitions the existing slice. We return its typed middle
    // after checking that no unaligned prefix or incomplete suffix exists.
    let (prefix, samples, suffix) = unsafe { v.align_to::<i16>() };
    assert!(
        prefix.is_empty() && suffix.is_empty(),
        "PCM bytes must be i16-aligned and contain complete samples"
    );
    samples
}

/// Releases a buffer back to the global pool.
#[inline]
pub fn release_buffer(v: PooledBuffer) {
    pool::get_pcm_pool().release(v);
}

/// Acquires a buffer from the global pool.
#[inline]
pub fn acquire_buffer(capacity: usize) -> PooledBuffer {
    pool::get_pcm_pool().acquire(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_byte_conversion_roundtrips_native_endian_samples() {
        let samples = vec![i16::MIN, -1, 0, 1, i16::MAX];
        let bytes = cast_to_bytes(samples.clone());
        assert_eq!(cast_from_bytes(bytes), samples);
    }

    #[test]
    fn pcm_byte_iterator_ignores_an_incomplete_trailing_sample() {
        let bytes = [1, 0, 2];
        assert_eq!(i16_samples(&bytes).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn aligned_pcm_bytes_can_be_borrowed_as_samples() {
        let samples = [i16::MIN, -1, 0, 1, i16::MAX];
        assert_eq!(as_i16_slice(as_byte_slice(&samples)), samples);
    }

    #[test]
    fn typed_pcm_pool_reuses_sample_capacity() {
        let pool = pool::get_pcm_pool();
        let buffer = pool.acquire(960);
        let capacity = buffer.capacity();
        pool.release(buffer);

        let reused = pool.acquire(960);
        assert_eq!(reused.capacity(), capacity);
    }
}
