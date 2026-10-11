use super::*;
#[test]
fn scalar_and_tall_balanced_energy() {
    for levels in [vec![2], vec![3], vec![2, 3], vec![4, 4, 4], vec![64]] {
        for mask in 1..1 << levels.len() {
            let b = basis(&levels, mask, &JobControl::default()).unwrap();
            let score = vec![1.; b.rank];
            let e = solve(&b, &vec![7; b.cells], &score, 0, 0., &JobControl::default())
                .unwrap()
                .unwrap();
            assert!((e.value.value() / (7. * b.rank as f64) - 1.).abs() < 1e-11);
        }
    }
}
#[test]
fn solver_refuses_uncertainty_and_honours_cancellation() {
    let c = JobControl::default();
    let b = basis(&[3], 1, &c).unwrap();
    assert!(solve(&b, &[1, 1, 1], &[0., 0.], 0, 1e-9, &c)
        .unwrap()
        .is_err());
    assert!(solve(&b, &[1, 1, 1], &[1., 1.], 0, 0.1, &c)
        .unwrap()
        .is_err());
    assert!(
        solve_with_iterations(&b, &[1, 1, 1], &[1., 1.], 0, 0., 0, &c)
            .unwrap()
            .is_err()
    );
    c.cancel();
    assert!(matches!(
        solve(&b, &[1, 1, 1], &[1., 1.], 0, 0., &c),
        Err(crate::error::AppError::Cancelled)
    ));
}
#[test]
fn bounded_nonconvergence_and_post_dependency_cancellation() {
    use crate::operation::CHECKPOINT_HOOK;
    use std::{cell::Cell, rc::Rc};
    let c = JobControl::default();
    let b = basis(&[64], 1, &c).unwrap();
    let counts = (1..=64).collect::<Vec<_>>();
    let scores = vec![1.; 63];
    let refused = solve_with_iterations(&b, &counts, &scores, 0, 0., 1, &c).unwrap();
    assert!(refused.is_err());
    let counter = Rc::new(Cell::new(0));
    let seen = counter.clone();
    let cancel = c.clone();
    let cut = 64 * 63 + 3;
    CHECKPOINT_HOOK.with(|hook| {
        *hook.borrow_mut() = Some(Box::new(move |_| {
            let n = seen.get() + 1;
            seen.set(n);
            if n == cut {
                cancel.cancel()
            }
        }))
    });
    let result = solve(&b, &counts, &scores, 0, 0., &c);
    CHECKPOINT_HOOK.with(|hook| *hook.borrow_mut() = None);
    assert!(matches!(result, Err(crate::error::AppError::Cancelled)));
    assert_eq!(counter.get(), cut);
}
