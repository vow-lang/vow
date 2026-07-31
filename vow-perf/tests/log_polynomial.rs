use vow_perf::{AnalysisError, ComplexityClass, Sample, Verdict, analyze};

fn instrumented_quadratic_logarithmic_work(input_size: u64) -> u64 {
    let mut operations = 0;

    for _ in 0..input_size {
        for _ in 0..input_size {
            let mut remaining = input_size;
            while remaining > 1 {
                operations += 1;
                remaining /= 2;
            }
        }
    }

    operations
}

#[test]
fn quadratic_declaration_rejects_instrumented_n_squared_log_n_work() {
    let samples = [16, 32, 64, 128, 256, 512].map(|input_size| {
        Sample::new(
            input_size,
            instrumented_quadratic_logarithmic_work(input_size),
        )
    });

    let analysis = analyze(ComplexityClass::Quadratic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Fail);
    assert_eq!(
        analysis.observed,
        Some(ComplexityClass::QuadraticLogarithmic)
    );
}

#[test]
fn canonicalization_extends_each_supported_polynomial_degree_with_one_log_factor() {
    assert_eq!(
        ComplexityClass::from_factors(0, 0),
        Ok(ComplexityClass::Constant)
    );
    assert_eq!(
        ComplexityClass::from_factors(0, 1),
        Ok(ComplexityClass::Logarithmic)
    );
    assert_eq!(
        ComplexityClass::from_factors(1, 0),
        Ok(ComplexityClass::Linear)
    );
    assert_eq!(
        ComplexityClass::from_factors(1, 1),
        Ok(ComplexityClass::Linearithmic)
    );
    assert_eq!(
        ComplexityClass::from_factors(2, 0),
        Ok(ComplexityClass::Quadratic)
    );
    assert_eq!(
        ComplexityClass::from_factors(2, 1),
        Ok(ComplexityClass::QuadraticLogarithmic)
    );
    assert_eq!(
        ComplexityClass::from_factors(3, 0),
        Ok(ComplexityClass::Cubic)
    );
    assert_eq!(
        ComplexityClass::from_factors(3, 1),
        Ok(ComplexityClass::CubicLogarithmic)
    );

    let polynomial_error = ComplexityClass::from_factors(4, 0).unwrap_err();
    assert_eq!(
        polynomial_error.to_string(),
        "polynomial degree 4 exceeds the cap of 3"
    );
    assert_eq!(
        ComplexityClass::from_factors(4, 1).unwrap_err(),
        polynomial_error
    );
    assert_eq!(
        ComplexityClass::from_factors(0, 2).unwrap_err().to_string(),
        "logarithmic degree 2 exceeds the cap of 1"
    );
}

#[test]
fn equal_quality_fits_choose_the_simplest_complexity_class() {
    let samples = [16, 32, 64, 128, 256, 512].map(|input_size| Sample::new(input_size, 0));

    let analysis = analyze(ComplexityClass::Constant, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::Constant));
}

#[test]
fn decreasing_work_does_not_fit_a_growing_complexity_class() {
    let samples = [16, 32, 64, 128, 256, 512]
        .map(|input_size| Sample::new(input_size, 100 - input_size.ilog2() as u64));

    let analysis = analyze(ComplexityClass::Constant, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Ambiguous);
    assert_eq!(analysis.observed, None);
}

#[test]
fn large_operation_baselines_preserve_small_growth_deltas() {
    let baseline = 1_u64 << 63;
    let samples = [16_u64, 32, 64, 128, 256, 512]
        .map(|input_size| Sample::new(input_size, baseline + input_size));

    let analysis = analyze(ComplexityClass::Constant, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Fail);
    assert_eq!(analysis.observed, Some(ComplexityClass::Linear));
}

#[test]
fn maximum_supported_declaration_does_not_pass_quartic_work() {
    let samples = [16_u64, 32, 64, 128, 256, 512]
        .map(|input_size| Sample::new(input_size, input_size.pow(4)));

    let analysis = analyze(ComplexityClass::CubicLogarithmic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Ambiguous);
    assert_eq!(analysis.observed, Some(ComplexityClass::CubicLogarithmic));
}

#[test]
fn maximum_supported_declaration_does_not_pass_cubic_log_squared_work() {
    let samples = [1024_u64, 2048, 4096, 8192, 16384, 32768].map(|input_size| {
        Sample::new(
            input_size,
            input_size.pow(3) * u64::from(input_size.ilog2()).pow(2),
        )
    });

    let analysis = analyze(ComplexityClass::CubicLogarithmic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Ambiguous);
    assert_eq!(analysis.observed, Some(ComplexityClass::CubicLogarithmic));
}

#[test]
fn maximum_supported_declaration_does_not_fail_thresholded_cubic_log_work() {
    let samples = [16_u64, 32, 64, 128, 256, 512, 1024, 2048].map(|input_size| {
        Sample::new(
            input_size,
            input_size.saturating_sub(100).pow(3) * u64::from(input_size.ilog2()),
        )
    });

    let analysis = analyze(ComplexityClass::CubicLogarithmic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Ambiguous);
    assert_eq!(analysis.observed, Some(ComplexityClass::CubicLogarithmic));
}

#[test]
fn maximum_supported_declaration_accepts_cubic_logarithmic_work() {
    let samples = [16_u64, 32, 64, 128, 256, 512]
        .map(|input_size| Sample::new(input_size, input_size.pow(3) * input_size.ilog2() as u64));

    let analysis = analyze(ComplexityClass::CubicLogarithmic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::CubicLogarithmic));
}

#[test]
fn maximum_supported_declaration_ignores_an_early_rising_residual() {
    let samples = [
        (16_u64, 1_u64),
        (32, 2),
        (64, 4),
        (128, 1),
        (256, 1),
        (512, 1),
    ]
    .map(|(input_size, early_multiplier)| {
        Sample::new(
            input_size,
            input_size.pow(3) * u64::from(input_size.ilog2()) * early_multiplier,
        )
    });

    let analysis = analyze(ComplexityClass::CubicLogarithmic, &samples).unwrap();

    assert_eq!(analysis.verdict, Verdict::Pass);
    assert_eq!(analysis.observed, Some(ComplexityClass::CubicLogarithmic));
}

#[test]
fn malformed_measurement_grids_are_rejected() {
    let too_few = analyze(
        ComplexityClass::Linear,
        &[Sample::new(16, 16), Sample::new(32, 32)],
    )
    .unwrap_err();
    assert_eq!(too_few, AnalysisError::TooFewSamples);
    assert_eq!(too_few.to_string(), "at least three samples are required");

    let too_small = analyze(
        ComplexityClass::Linear,
        &[Sample::new(1, 1), Sample::new(2, 2), Sample::new(4, 4)],
    )
    .unwrap_err();
    assert_eq!(
        too_small,
        AnalysisError::InputSizeTooSmall {
            index: 0,
            input_size: 1
        }
    );
    assert_eq!(
        too_small.to_string(),
        "sample 0 has input size 1; sizes must be at least 2"
    );

    let non_increasing = analyze(
        ComplexityClass::Linear,
        &[
            Sample::new(16, 16),
            Sample::new(16, 32),
            Sample::new(32, 64),
        ],
    )
    .unwrap_err();
    assert_eq!(
        non_increasing,
        AnalysisError::NonIncreasingInputSize {
            index: 1,
            previous: 16,
            input_size: 16
        }
    );
    assert_eq!(
        non_increasing.to_string(),
        "sample 1 has input size 16 after 16; sizes must increase"
    );
}

#[test]
fn log_polynomial_classes_have_class_specific_doubling_ratios() {
    let expected = [
        (ComplexityClass::Constant, 1.0),
        (ComplexityClass::Logarithmic, 1.1),
        (ComplexityClass::Linear, 2.0),
        (ComplexityClass::Linearithmic, 2.2),
        (ComplexityClass::Quadratic, 4.0),
        (ComplexityClass::QuadraticLogarithmic, 4.4),
        (ComplexityClass::Cubic, 8.0),
        (ComplexityClass::CubicLogarithmic, 8.8),
    ];

    for (class, expected_ratio) in expected {
        let actual = class.expected_doubling_ratio(1024).unwrap();
        assert!(
            (actual - expected_ratio).abs() < 1e-12,
            "{class:?}: expected {expected_ratio}, got {actual}"
        );
    }
}

#[test]
fn doubling_ratios_reject_sizes_below_the_measurement_domain() {
    let error = ComplexityClass::Linear
        .expected_doubling_ratio(1)
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "input size 1 is too small for a doubling ratio"
    );
}
