//! Empirical complexity classification for Vow performance contracts.
//!
//! This crate owns the fixed single-variable candidate set, the pure
//! statistical analysis core, and operation-count instrumentation for a
//! dedicated performance-test artifact.
//!
//! [`instrument_module`] accepts immutable typed IR, clones it, and uses
//! [`vow_ir::InsertionSet`] only on the clone. Production codegen must continue
//! to consume the original module; the returned [`InstrumentedModule`] is a
//! separate compilation input. Each inserted counter call receives a fresh
//! function-unique instruction ID, while existing IDs and references remain
//! unchanged. Consequently, instrumented IR retains deterministic
//! `encode -> decode -> encode` stability without involving the source-level
//! `parse -> print -> parse` canonical-form invariant.
//!
//! Input generation and CLI integration remain separate concerns tracked by
//! the `vow-perf` implementation roadmap.

mod instrumentation;

use std::fmt;

pub use instrumentation::{InstrumentationError, InstrumentedModule, instrument_module};

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
/// tail is still rising or has too few intervals to establish a trend, because
/// finite samples cannot distinguish a higher-order curve from lower-order
/// effects with certainty.
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
/// Tuned against today's exact synthetic fixtures, where any non-decreasing
/// step counts as rising; will need revisiting once real (noisy) instrumented
/// operation counts feed [`analyze`].
const NORMALIZED_TREND_TOLERANCE: f64 = 1.0e-9;
const REQUIRED_RISING_SLOPE_STEPS: usize = 2;
const REQUIRED_MAXIMUM_TREND_SAMPLES: usize = REQUIRED_RISING_SLOPE_STEPS + 2;
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
const MAXIMUM_CANDIDATE: ComplexityClass = CANDIDATES[CANDIDATES.len() - 1];

/// Classify measured operation counts against a declared complexity class.
///
/// Assumes `samples` is a well-ranged measurement grid: best-fit selection
/// compares candidates across the whole sample set, so a grid whose low end
/// sits inside a function's pre-threshold plateau can make an adjacent
/// higher-order class narrowly out-fit the correct one, producing a false
/// `Fail`. [`recommended_grid`] generates a grid wide enough to resolve this
/// for the threshold magnitudes this crate has validated against; callers
/// building their own grid should prefer a similarly wide, high-end-heavy
/// range over a narrow one.
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

    let verdict = if observed == MAXIMUM_CANDIDATE
        && declared == MAXIMUM_CANDIDATE
        && maximum_trend_is_ambiguous(samples)
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

fn maximum_trend_is_ambiguous(samples: &[Sample]) -> bool {
    if samples.len() < REQUIRED_MAXIMUM_TREND_SAMPLES {
        return true;
    }

    let mut slopes = samples
        .windows(2)
        .rev()
        .map(|pair| normalized_interval_slope(MAXIMUM_CANDIDATE, &pair[0], &pair[1]));
    let mut newer = slopes.next().expect("sample count checked above");

    for _ in 0..REQUIRED_RISING_SLOPE_STEPS {
        let older = slopes.next().expect("sample count checked above");
        let scale = newer.abs().max(older.abs());
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

/// Number of geometrically-doubled sizes in `recommended_grid`'s output.
const RECOMMENDED_GRID_SAMPLE_COUNT: u32 = 12;

/// An invalid `min_input_size` for [`recommended_grid`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecommendedGridError {
    InputSizeTooSmall { min_input_size: u64 },
    Overflow { min_input_size: u64 },
}

impl fmt::Display for RecommendedGridError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputSizeTooSmall { min_input_size } => write!(
                formatter,
                "input size {min_input_size} is too small for a measurement grid"
            ),
            Self::Overflow { min_input_size } => write!(
                formatter,
                "input size {min_input_size} is too large to double \
                 {} times without overflow",
                RECOMMENDED_GRID_SAMPLE_COUNT - 1
            ),
        }
    }
}

impl std::error::Error for RecommendedGridError {}

/// A default measurement grid for [`analyze`]'s input.
///
/// Returns twelve sizes, geometrically doubling from `min_input_size` (so
/// `min_input_size = 16` yields `[16, 32, ..., 32768]`). Empirically validated
/// at `min_input_size = 16` to resolve false `Fail`s caused by a flat
/// pre-threshold cost region up to 200 units wide, for every supported
/// polynomial degree, with no change in verdict for genuine complexity
/// violations.
///
/// This is a mitigation, not a guarantee: it is bounded by how far the grid's
/// high end sits above the workload's hidden threshold. A function whose
/// flat region extends past this grid's high end needs a wider grid than
/// this default provides — see `analyze`'s doc comment. The floor matters
/// too: a smaller `min_input_size` adds more samples inside the plateau and
/// can still produce a false `Fail` for the same threshold widths this
/// validation covers — prefer 16 unless a larger floor is already known to
/// clear the workload's threshold.
pub fn recommended_grid(min_input_size: u64) -> Result<Vec<u64>, RecommendedGridError> {
    if min_input_size < 2 {
        return Err(RecommendedGridError::InputSizeTooSmall { min_input_size });
    }

    let mut size = min_input_size;
    let mut sizes = Vec::with_capacity(RECOMMENDED_GRID_SAMPLE_COUNT as usize);
    sizes.push(size);
    for _ in 1..RECOMMENDED_GRID_SAMPLE_COUNT {
        size = size
            .checked_mul(2)
            .ok_or(RecommendedGridError::Overflow { min_input_size })?;
        sizes.push(size);
    }
    Ok(sizes)
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

#[cfg(test)]
mod tests {
    use super::{CANDIDATES, ComplexityClass};

    /// Derives the full class set from `from_factors` (rather than hand-listing
    /// variants a second time) so that raising its degree caps for a future
    /// variant makes this test fail until `CANDIDATES` is updated to match.
    #[test]
    fn candidates_cover_every_class_from_factors_can_produce_in_ascending_order() {
        let mut derived = Vec::new();
        let mut polynomial_degree = 0u8;
        loop {
            let mut logarithmic_degree = 0u8;
            let mut produced_any = false;
            while let Ok(class) =
                ComplexityClass::from_factors(polynomial_degree, logarithmic_degree)
            {
                derived.push(class);
                produced_any = true;
                logarithmic_degree += 1;
            }
            if !produced_any {
                break;
            }
            polynomial_degree += 1;
        }

        let mut sorted_derived = derived.clone();
        sorted_derived.sort();

        assert_eq!(
            derived.len(),
            CANDIDATES.len(),
            "CANDIDATES must list exactly the classes from_factors can produce"
        );
        assert_eq!(
            sorted_derived,
            CANDIDATES.to_vec(),
            "CANDIDATES must be in ascending ComplexityClass order with no gaps or duplicates"
        );
    }
}
