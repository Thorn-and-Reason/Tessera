use super::*;
use crate::data::{Budget, MAX_OWNED_BYTES};

// Catches dropping caller/history bytes before allocating validation indexes.
#[test]
fn retained_bytes_refuse_before_growth() {
    let data = dataset();
    let retained = data.clone();
    let request = numeric_request();
    let baseline = Budget::dataset_bytes(&[&data, &retained], 0).unwrap();
    let extra = MAX_OWNED_BYTES - baseline;
    assert!(matches!(
        request.validate_current(&data, &[&retained], extra),
        Err(AppError::SizeLimit { .. })
    ));
    request
        .validate_current(&data, &[&retained], request.heap_bytes().unwrap())
        .unwrap();
    assert!(matches!(
        request.validate_current(&data, &[], usize::MAX),
        Err(AppError::SizeLimit { .. })
    ));
    let (rows, _) = selected(&data, &request);
    assert!(matches!(
        coding::discover(
            &data,
            &request,
            &rows,
            &JobControl::default(),
            MAX_OWNED_BYTES
        ),
        Err(AppError::SizeLimit { .. })
    ));
    assert_eq!(rows.len(), 4);
}

// Catches counting lengths instead of simultaneous reserved capacities and nested strings.
#[test]
fn request_and_result_count_nested_capacity() {
    let mut request = category_request(Cell::Text(String::with_capacity(4096)));
    let ordinary = request.heap_bytes().unwrap();
    request.predictors.reserve_exact(20);
    assert!(
        request.heap_bytes().unwrap() >= ordinary + 20 * std::mem::size_of::<RegressionPredictor>()
    );
    assert!(ordinary >= 4096);
    let result = fitted_fixture();
    let ordinary = result.owned_bytes().unwrap();
    let mut retained = result;
    let RegressionOutcome::Fitted(fit) = &mut retained.outcome else {
        unreachable!()
    };
    fit.coefficients.reserve_exact(10);
    fit.residual_diagnostics.qq_points.reserve_exact(10);
    fit.coefficients[0]
        .t
        .reason
        .as_mut()
        .unwrap()
        .reserve_exact(4096);
    assert!(
        retained.owned_bytes().unwrap()
            >= ordinary
                + 10 * std::mem::size_of::<RegressionCoefficient>()
                + 10 * std::mem::size_of::<RegressionQqPoint>()
                + 4096
    );
}

// Catches charging only category values, omitting repeated term/reference strings.
#[test]
fn coding_owners_include_exact_cells_and_contrast_copies() {
    let data = dataset();
    let request = category_request(Cell::Text("a".into()));
    let (rows, _) = selected(&data, &request);
    let (levels, terms, failure) = coding::discover(
        &data,
        &request,
        &rows,
        &JobControl::default(),
        coding_baseline(&data, &request, &rows),
    )
    .unwrap();
    assert!(failure.is_none());
    let expected = std::mem::size_of::<RegressionLevels>()
        + 64 * std::mem::size_of::<RegressionTerm>()
        + 2 * std::mem::size_of::<Cell>()
        + 4;
    assert_eq!(
        super::super::accounting::coding_bytes(&levels, &terms).unwrap(),
        expected
    );
    let mut grown = terms;
    let RegressionTerm::Contrast {
        reference: Cell::Text(reference),
        ..
    } = &mut grown[1]
    else {
        unreachable!()
    };
    reference.reserve_exact(4096);
    assert_eq!(
        super::super::accounting::coding_bytes(&levels, &grown).unwrap(),
        expected + 4096
    );
}
