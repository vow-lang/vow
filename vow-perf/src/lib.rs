//! Empirical complexity classification for Vow performance contracts.
//!
//! This crate owns the fixed single-variable candidate set and the pure
//! statistical analysis core. IR instrumentation, input generation, and CLI
//! integration are separate concerns tracked by the `vow-perf` implementation
//! roadmap.

use std::fmt;

/// A canonical single-variable complexity class.
///
/// Variant order is asymptotic order and drives the derived comparison used by
/// [`analyze`]. Keep it aligned with the fixed ordering in the performance
/// guarantees design.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ComplexityClass {
    Constant,
    Logarithmic,
    Linear,
    Linearithmic,
    Quadratic,
    QuadraticLogarithmic,
    Cubic,
    CubicLogarithmic,
}

impl ComplexityClass {
    /// Canonicalize repeated polynomial and logarithmic factors.
    pub const fn from_factors(
        polynomial_degree: u8,
        logarithmic_degree: u8,
    ) -> Result<Self, ComplexityClassError> {
        if polynomial_degree > 3 {
            return Err(ComplexityClassError::PolynomialDegreeTooHigh {
                degree: polynomial_degree,
            });
        }
        if logarithmic_degree > 1 {
            return Err(ComplexityClassError::LogarithmicDegreeTooHigh {
                degree: logarithmic_degree,
            });
        }

        match (polynomial_degree, logarithmic_degree) {
            (0, 0) => Ok(Self::Constant),
            (0, 1) => Ok(Self::Logarithmic),
            (1, 0) => Ok(Self::Linear),
            (1, 1) => Ok(Self::Linearithmic),
            (2, 0) => Ok(Self::Quadratic),
            (2, 1) => Ok(Self::QuadraticLogarithmic),
            (3, 0) => Ok(Self::Cubic),
            (3, 1) => Ok(Self::CubicLogarithmic),
            _ => unreachable!(),
        }
    }

    /// Return the expected `T(2n) / T(n)` ratio for this class.
    pub fn expected_doubling_ratio(self, input_size: u64) -> Result<f64, DoublingRatioError> {
        if input_size < 2 {
            return Err(DoublingRatioError::InputSizeTooSmall { input_size });
        }

        let (polynomial_degree, logarithmic_degree) = self.factor_degrees();
        let polynomial_ratio = 2_f64.powi(i32::from(polynomial_degree));
        if logarithmic_degree == 0 {
            return Ok(polynomial_ratio);
        }

        let n = input_size as f64;
        Ok(polynomial_ratio * (2.0 * n).log2() / n.log2())
    }

    const fn factor_degrees(self) -> (u8, u8) {
        match self {
            Self::Constant => (0, 0),
            Self::Logarithmic => (0, 1),
            Self::Linear => (1, 0),
            Self::Linearithmic => (1, 1),
            Self::Quadratic => (2, 0),
            Self::QuadraticLogarithmic => (2, 1),
            Self::Cubic => (3, 0),
            Self::CubicLogarithmic => (3, 1),
        }
    }
}

/// A complexity expression outside the fixed canonical class set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComplexityClassError {
    PolynomialDegreeTooHigh { degree: u8 },
    LogarithmicDegreeTooHigh { degree: u8 },
}

impl fmt::Display for ComplexityClassError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PolynomialDegreeTooHigh { degree } => {
                write!(formatter, "polynomial degree {degree} exceeds the cap of 3")
            }
            Self::LogarithmicDegreeTooHigh { degree } => {
                write!(
                    formatter,
                    "logarithmic degree {degree} exceeds the cap of 1"
                )
            }
        }
    }
}

impl std::error::Error for ComplexityClassError {}

/// An input size for which a doubling ratio is undefined.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoublingRatioError {
    InputSizeTooSmall { input_size: u64 },
}

impl fmt::Display for DoublingRatioError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputSizeTooSmall { input_size } => {
                write!(
                    formatter,
                    "input size {input_size} is too small for a doubling ratio"
                )
            }
        }
    }
}

impl std::error::Error for DoublingRatioError {}

/// One operation-count measurement at a controlled input size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sample {
    pub input_size: u64,
    pub operations: u64,
}

impl Sample {
    pub const fn new(input_size: u64, operations: u64) -> Self {
        Self {
            input_size,
            operations,
        }
    }
}

/// Classification of measured growth against a declared upper bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Pass,
    Fail,
    Ambiguous,
}

/// Result of fitting measured operation counts to the fixed candidate set.
///
/// `observed` is `None` when no candidate meets the fit threshold. An
/// `Ambiguous` result may retain the maximum candidate when its normalized
/// tail is still rising, because finite samples cannot distinguish a
/// higher-order curve from lower-order effects with certainty.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Analysis {
    pub verdict: Verdict,
    pub observed: Option<ComplexityClass>,
}

/// Invalid measurement data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnalysisError {
    TooFewSamples,
    InputSizeTooSmall {
        index: usize,
        input_size: u64,
    },
    NonIncreasingInputSize {
        index: usize,
        previous: u64,
        input_size: u64,
    },
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewSamples => formatter.write_str("at least three samples are required"),
            Self::InputSizeTooSmall { index, input_size } => write!(
                formatter,
                "sample {index} has input size {input_size}; sizes must be at least 2"
            ),
            Self::NonIncreasingInputSize {
                index,
                previous,
                input_size,
            } => write!(
                formatter,
                "sample {index} has input size {input_size} after {previous}; sizes must increase"
            ),
        }
    }
}

impl std::error::Error for AnalysisError {}

const MINIMUM_R_SQUARED: f64 = 0.90;
const NORMALIZED_TREND_TOLERANCE: f64 = 1.0e-9;
const REQUIRED_RISING_SLOPE_STEPS: usize = 2;
const CANDIDATES: [ComplexityClass; 8] = [
    ComplexityClass::Constant,
    ComplexityClass::Logarithmic,
    ComplexityClass::Linear,
    ComplexityClass::Linearithmic,
    ComplexityClass::Quadratic,
    ComplexityClass::QuadraticLogarithmic,
    ComplexityClass::Cubic,
    ComplexityClass::CubicLogarithmic,
];

/// Classify measured operation counts against a declared complexity class.
pub fn analyze(declared: ComplexityClass, samples: &[Sample]) -> Result<Analysis, AnalysisError> {
    if samples.len() < 3 {
        return Err(AnalysisError::TooFewSamples);
    }
    for (index, sample) in samples.iter().enumerate() {
        if sample.input_size < 2 {
            return Err(AnalysisError::InputSizeTooSmall {
                index,
                input_size: sample.input_size,
            });
        }
        if index > 0 && sample.input_size <= samples[index - 1].input_size {
            return Err(AnalysisError::NonIncreasingInputSize {
                index,
                previous: samples[index - 1].input_size,
                input_size: sample.input_size,
            });
        }
    }

    let mut fits = CANDIDATES
        .into_iter()
        .map(|candidate| (candidate, r_squared(candidate, samples)));
    let first_fit = fits.next().expect("fixed candidate set is non-empty");
    let (observed, best_r_squared) = fits.fold(first_fit, |best, candidate| {
        if candidate.1 > best.1 {
            candidate
        } else {
            best
        }
    });

    if best_r_squared < MINIMUM_R_SQUARED {
        return Ok(Analysis {
            verdict: Verdict::Ambiguous,
            observed: None,
        });
    }

    let verdict = if observed == ComplexityClass::CubicLogarithmic
        && observed <= declared
        && has_rising_maximum_residual_tail(samples)
    {
        Verdict::Ambiguous
    } else if observed <= declared {
        Verdict::Pass
    } else {
        Verdict::Fail
    };

    Ok(Analysis {
        verdict,
        observed: Some(observed),
    })
}

fn has_rising_maximum_residual_tail(samples: &[Sample]) -> bool {
    let maximum = ComplexityClass::CubicLogarithmic;
    let mut slopes = samples
        .windows(2)
        .rev()
        .map(|pair| normalized_interval_slope(maximum, &pair[0], &pair[1]));
    let Some(mut newer) = slopes.next() else {
        return false;
    };

    for _ in 0..REQUIRED_RISING_SLOPE_STEPS {
        let Some(older) = slopes.next() else {
            return false;
        };
        let scale = newer.abs().max(older.abs()).max(1.0);
        if newer <= older + scale * NORMALIZED_TREND_TOLERANCE {
            return false;
        }
        newer = older;
    }

    true
}

fn normalized_interval_slope(class: ComplexityClass, previous: &Sample, current: &Sample) -> f64 {
    let operation_delta = (i128::from(current.operations) - i128::from(previous.operations)) as f64;
    let basis_delta =
        basis_value(class, current.input_size) - basis_value(class, previous.input_size);
    operation_delta / basis_delta
}

fn r_squared(class: ComplexityClass, samples: &[Sample]) -> f64 {
    let count = samples.len() as f64;
    // Linear regression is translation-invariant. Subtracting before the
    // conversion preserves small deltas when counters have a large baseline.
    let operation_origin = i128::from(samples[0].operations);
    let operation_delta =
        |sample: &Sample| (i128::from(sample.operations) - operation_origin) as f64;
    let mean_x = samples
        .iter()
        .map(|sample| basis_value(class, sample.input_size))
        .sum::<f64>()
        / count;
    let mean_y = samples.iter().map(operation_delta).sum::<f64>() / count;

    let (covariance, variance_x) = samples.iter().fold((0.0, 0.0), |acc, sample| {
        let centered_x = basis_value(class, sample.input_size) - mean_x;
        let centered_y = operation_delta(sample) - mean_y;
        (
            acc.0 + centered_x * centered_y,
            acc.1 + centered_x * centered_x,
        )
    });

    if variance_x == 0.0 {
        return if samples
            .iter()
            .all(|sample| sample.operations == samples[0].operations)
        {
            1.0
        } else {
            0.0
        };
    }

    let slope = covariance / variance_x;
    if slope < 0.0 {
        return 0.0;
    }
    let intercept = mean_y - slope * mean_x;
    let (residual_sum, total_sum) = samples.iter().fold((0.0, 0.0), |acc, sample| {
        let actual = operation_delta(sample);
        let predicted = intercept + slope * basis_value(class, sample.input_size);
        (
            acc.0 + (actual - predicted).powi(2),
            acc.1 + (actual - mean_y).powi(2),
        )
    });

    if total_sum == 0.0 {
        return if residual_sum == 0.0 { 1.0 } else { 0.0 };
    }

    1.0 - residual_sum / total_sum
}

fn basis_value(class: ComplexityClass, input_size: u64) -> f64 {
    let n = input_size as f64;
    let log_n = n.log2();

    match class {
        ComplexityClass::Constant => 1.0,
        ComplexityClass::Logarithmic => log_n,
        ComplexityClass::Linear => n,
        ComplexityClass::Linearithmic => n * log_n,
        ComplexityClass::Quadratic => n.powi(2),
        ComplexityClass::QuadraticLogarithmic => n.powi(2) * log_n,
        ComplexityClass::Cubic => n.powi(3),
        ComplexityClass::CubicLogarithmic => n.powi(3) * log_n,
    }
}
