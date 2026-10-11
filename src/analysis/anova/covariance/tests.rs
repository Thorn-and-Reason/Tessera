use super::*;
#[test]
fn spherical_singular_zero_and_structural_rank() {
    let a = evaluate(&[2., 0., 0., 2.], 2, 10, 3).unwrap();
    assert_eq!(a.epsilon, 1.);
    let b = evaluate(&[4., 0., 0., 0.], 2, 3, 3).unwrap();
    assert_eq!(b.epsilon, 0.5);
    assert!(evaluate(&[0.; 4], 2, 3, 3).is_err());
    for d in [1, 2, 3, 7, 49, 63] {
        let mut gram = vec![0.; d * d];
        gram[0] = 1.;
        let a = evaluate(&gram, d, 2, d + 1).unwrap();
        assert_eq!(a.epsilon, 1. / d as f64);
        assert_eq!(a.df1, 1.);
        assert_eq!(a.df2, 1.);
    }
}
#[test]
fn physical_bound_and_nonfinite_refuse() {
    assert!(evaluate(&[1., 0., 0., 1.], 2, 2, 3).is_err());
    assert!(evaluate(&[f64::NAN], 1, 3, 2).is_err());
    assert!(evaluate(&[-1.], 1, 3, 2).is_err());
    let a = evaluate(&[3.], 1, 8, 2).unwrap();
    assert_eq!((a.epsilon, a.df1, a.df2), (1., 1., 7.));
}
#[test]
fn n2_known_rank_one_survives_noisy_epsilon_but_rank_two_refuses() {
    let x = [0.375, 0.21650635094610965];
    let gram = [x[0] * x[0], x[0] * x[1], x[1] * x[0], x[1] * x[1]];
    let v = evaluate(&gram, 2, 2, 3).unwrap();
    assert_eq!((v.epsilon, v.df1, v.df2), (0.5, 1., 1.));
    assert!(evaluate(&[1., 0., 0., 1.], 2, 2, 3).is_err());
}
