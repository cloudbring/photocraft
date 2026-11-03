//! In-place order statistics for reusable sample buffers.

/// Returns the upper median, or `None` for an empty slice, rearranging the samples in place.
/// Even lengths use the element at `len / 2`, not the average of the middle pair.
/// Values, including signed zero and NaNs (with their signs and payloads), follow `f32::total_cmp`.
pub fn median_f32(samples: &mut [f32]) -> Option<f32> {
    if samples.is_empty() {
        return None;
    }
    let middle = samples.len() / 2;
    Some(*samples.select_nth_unstable_by(middle, f32::total_cmp).1)
}

/// Returns the upper median under `f64::total_cmp`, or `None` for an empty slice.
/// Like [`median_f32`], this preserves the ordering of signed zeros and all NaN bit patterns.
pub fn median_f64(samples: &mut [f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let middle = samples.len() / 2;
    Some(*samples.select_nth_unstable_by(middle, f64::total_cmp).1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_matches_sorted_total_order() {
        let mut state = 0x1234_5678_9abc_def0u64;
        for len in 0..300 {
            for _ in 0..20 {
                let mut values = Vec::with_capacity(len);
                for _ in 0..len {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    values.push(match state % 8 {
                        0 => f32::from_bits(0x7fc0_0000 | (state as u32 & 0xffff)),
                        1 => f32::from_bits(0xffc0_0000 | (state as u32 & 0xffff)),
                        2 => -0.0,
                        3 => 0.0,
                        4 => f32::INFINITY,
                        5 => f32::NEG_INFINITY,
                        _ => (state % 17) as f32,
                    });
                }
                let mut sorted = values.clone();
                sorted.sort_by(f32::total_cmp);
                let expected = sorted.get(len / 2).map(|value| value.to_bits());
                assert_eq!(median_f32(&mut values).map(f32::to_bits), expected);

                let mut doubles: Vec<f64> = sorted.iter().map(|&value| value as f64).collect();
                doubles.reverse();
                let mut expected_doubles = doubles.clone();
                expected_doubles.sort_by(f64::total_cmp);
                assert_eq!(median_f64(&mut doubles).map(f64::to_bits), expected_doubles.get(len / 2).map(|value| value.to_bits()));
            }
        }
    }
}
