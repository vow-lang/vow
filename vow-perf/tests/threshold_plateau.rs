use vow_perf::{ComplexityClass, RecommendedGridError, Sample, Verdict, analyze, recommended_grid};

#[test]
fn recommended_grid_doubles_from_the_given_floor() {
    assert_eq!(
        recommended_grid(16).unwrap(),
        vec![
            16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768
        ]
    );
}

#[test]
fn recommended_grid_rejects_floors_below_the_measurement_domain() {
    let error = recommended_grid(1).unwrap_err();

    assert_eq!(
        error,
        RecommendedGridError::InputSizeTooSmall { min_input_size: 1 }
    );
    assert_eq!(
        error.to_string(),
        "input size 1 is too small for a measurement grid"
    );
}

#[test]
fn recommended_grid_rejects_floors_that_would_overflow_while_doubling() {
    let error = recommended_grid(u64::MAX).unwrap_err();

    assert_eq!(
        error,
        RecommendedGridError::Overflow {
            min_input_size: u64::MAX
        }
    );
    assert_eq!(
        error.to_string(),
        "input size 18446744073709551615 is too large to double 11 times without overflow"
    );
}

#[test]
fn linear_threshold_plateau_is_no_longer_a_false_fail_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size.saturating_sub(100)))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Linear, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Linear));
}

#[test]
fn quadratic_threshold_plateau_is_no_longer_a_false_fail_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size.saturating_sub(100).pow(2)))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Quadratic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Quadratic));
}

#[test]
fn cubic_threshold_plateau_is_no_longer_a_false_fail_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size.saturating_sub(100).pow(3)))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Cubic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Cubic));
}

#[test]
fn linear_wider_threshold_plateau_is_no_longer_a_false_fail_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size.saturating_sub(200)))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Linear, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Linear));
}

#[test]
fn quadratic_wider_threshold_plateau_is_no_longer_a_false_fail_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size.saturating_sub(200).pow(2)))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Quadratic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Quadratic));
}

#[test]
fn cubic_wider_threshold_plateau_is_no_longer_a_false_fail_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size.saturating_sub(200).pow(3)))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Cubic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Cubic));
}

#[test]
fn linear_log_violation_still_fails_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| Sample::new(input_size, input_size * u64::from(input_size.ilog2())))
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Linear, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Fail);
    assert_eq!(analysis.observed, Some(ComplexityClass::Linearithmic));
}

#[test]
fn quadratic_log_violation_still_fails_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| {
            Sample::new(
                input_size,
                input_size.pow(2) * u64::from(input_size.ilog2()),
            )
        })
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Quadratic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Fail);
    assert_eq!(
        analysis.observed,
        Some(ComplexityClass::QuadraticLogarithmic)
    );
}

#[test]
fn cubic_log_violation_still_fails_on_the_recommended_grid() {
    let samples = recommended_grid(16)
        .unwrap()
        .into_iter()
        .map(|input_size| {
            Sample::new(
                input_size,
                input_size.pow(3) * u64::from(input_size.ilog2()),
            )
        })
        .collect::<Vec<_>>();

    let analysis = analyze(ComplexityClass::Cubic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Fail);
    assert_eq!(analysis.observed, Some(ComplexityClass::CubicLogarithmic));
}
