use std::future::Future;

// Match Komari's TCP policy: only a successful, slow handshake is rechecked.
// A large drop suggests a retransmission; it is a failed round, not a lower RTT.
pub(crate) async fn measure_tcp<F, T>(mut probe: F) -> Option<f32>
where
    F: FnMut() -> T,
    T: Future<Output = Option<f32>>,
{
    let first = probe().await?;
    if first <= 1000.0 {
        return Some(first);
    }
    for _ in 0..3 {
        let next = probe().await?;
        if next <= 1000.0 {
            return (first - next <= 800.0).then_some(next);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::ready;
    use std::task::{Context, Poll, Waker};

    fn check(values: &[Option<f32>], expected: Option<f32>, expected_calls: usize) {
        let mut values = values.iter().copied();
        let mut calls = 0;
        let result = {
            let future = measure_tcp(|| {
                calls += 1;
                ready(values.next().expect("unexpected extra probe"))
            });
            let mut future = std::pin::pin!(future);
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
        };
        assert_eq!(result, Poll::Ready(expected));
        assert_eq!(calls, expected_calls);
    }

    #[test]
    fn successful_normal_samples_are_not_retried() {
        for value in [0.0, 330.0, 1000.0] {
            check(&[Some(value)], Some(value), 1);
        }
    }

    #[test]
    fn initial_failures_are_not_retried_or_replaced() {
        check(&[None], None, 1);
    }

    #[test]
    fn high_latency_can_recover_within_three_rechecks() {
        check(&[Some(1001.0), Some(900.0)], Some(900.0), 2);
        check(
            &[Some(1200.0), Some(1100.0), Some(1050.0), Some(900.0)],
            Some(900.0),
            4,
        );
    }

    #[test]
    fn retransmission_sized_drops_are_failed_rounds() {
        check(&[Some(1300.0), Some(330.0)], None, 2);
        check(&[Some(1200.0), Some(1100.0), Some(300.0)], None, 3);
        check(&[Some(1200.0), Some(400.0)], Some(400.0), 2);
        check(&[Some(1201.0), Some(400.0)], None, 2);
    }

    #[test]
    fn persistent_high_latency_is_bounded_and_recorded_as_failure() {
        check(
            &[Some(1100.0), Some(1200.0), Some(1300.0), Some(1400.0)],
            None,
            4,
        );
    }

    #[test]
    fn recheck_errors_fail_the_round_immediately() {
        check(&[Some(1300.0), None], None, 2);
        check(&[Some(1300.0), Some(1100.0), None], None, 3);
    }
}
