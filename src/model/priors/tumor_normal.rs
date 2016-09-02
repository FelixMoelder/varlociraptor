use std::f64;

use itertools::{Itertools, linspace};
use ordered_float::NotNaN;
use bio::stats::LogProb;

use model::{Variant, ContinousAlleleFreqs, DiscreteAlleleFreqs, AlleleFreq};

use priors::InfiniteSitesNeutralVariationModel;
use priors::{PairModel, Model};


/// Tumor-normal prior model using ploidy, heterozygosity (in normal tissue) and tumor mutation rate
/// per effective cell division.
/// The latter is the quotient mu/beta, with mu being the mutation rate and beta being the fraction
/// of effective cell divisions (both lineages survive). Alone, the parameters are not observable.
/// However, mu/beta can be estimated from e.g. SNV calls. It is the slope of the linear model
/// `y = mu/beta * (x -  1 / fmax)``, with `x` being the reciprocal of the observed allele frequencies
/// and y being the number of observed mutations corresponding to each frequency
/// (see Williams et al. Nature Genetics 2016).
///
/// Based on the Williams model, the tail probability of a somatic allele frequency F > f can be expressed
/// as
/// `Pr(F > f) = M(f) / n = mu/beta (1 / f - 1 / fmax) / n`
/// with `n` being the size of the genome and `fmax` is the expected allele frequency of clonal variants
/// at the beginning of tumor evolution.
/// From this, we can obtain the cumulative distribution function as `Pr(F <= f) = 1 - Pr(F > f)`.
/// Consequently, the density becomes the first derivative, i.e. `Pr(F = f) = - M(f)' / n = mu/beta * 1/n * 1/f²` for f>=fmin
/// with `fmin = sqrt(mu/beta * 1/n)`.
///
/// The prior probability for a germline allele frequency f (e.g. 0.0, 0.5, 1.0) in the tumor is
/// calculated with an `InfiniteSitesNeutralVariationModel`. This is valid since clonal variants
/// come from the underlying normal tissue and Williams model assumes that allele frequencies
/// do not change during tumor evolution (no genetic drift, no selection).
///
/// For the final prior, we consider a given tumor purity and calculate the combined prior
/// for all possible allele frequency combinations satisfying `af = purity * af_tumor + (1-purity) * af_normal`.
pub struct TumorNormalModel {
    normal_model: InfiniteSitesNeutralVariationModel,
    effective_mutation_rate: f64,
    deletion_factor: f64,
    insertion_factor: f64,
    genome_size: u64,
    allele_freqs_tumor: ContinousAlleleFreqs,
    grid_points: usize,
    af_min: AlleleFreq
}


impl TumorNormalModel {
    /// Create new model.
    ///
    /// # Arguments
    ///
    /// * `ploidy` - the ploidy in the corresponding normal sample (e.g. 2 for diploid)
    /// * `effective_mutation_rate` - the mutation rate per effective cell division in the tumor
    /// * `genome_size` - the size of the genome
    /// * `heterozygosity` - expected heterozygosity in the corresponding normal
    pub fn new(
        ploidy: u32,
        effective_mutation_rate: f64,
        deletion_factor: f64,
        insertion_factor: f64,
        genome_size: u64,
        heterozygosity: f64) -> Self {
        assert!(effective_mutation_rate < genome_size as f64);
        let af_min = AlleleFreq((effective_mutation_rate / genome_size as f64).sqrt());

        TumorNormalModel {
            normal_model: InfiniteSitesNeutralVariationModel::new(ploidy, heterozygosity),
            effective_mutation_rate: effective_mutation_rate,
            deletion_factor: deletion_factor,
            insertion_factor: insertion_factor,
            genome_size: genome_size,
            allele_freqs_tumor: AlleleFreq(0.0)..AlleleFreq(1.0),
            grid_points: 51,
            af_min: af_min
        }
    }

    fn somatic_prior_prob(&self, af_tumor: AlleleFreq, af_normal: AlleleFreq, variant: Variant) -> LogProb {
        // af_tumor = af_normal + af_somatic
        // this yields for af_somatic
        let af_somatic = af_tumor - af_normal;
        // af_somatic can become negative, meaning that at some point a variant from normal was lost
        // in one cell (LOH!!). Again, the frequency corresponds to time in tumor evolution since the model
        // assumes that all frequencies stay constant. Hence, we can simply take the absolute value
        // of af_somatic here. This is equivalent to calculating
        // af_tumor = af_normal - af_somatic
        // for that case.
        let af_somatic = af_somatic.abs();

        // mu/beta * 1 / (af**2 * n)
        if af_somatic <= *self.af_min {
            return LogProb::ln_one();
        }

        // adjust effective mutation rate by type-specific factor
        let factor = match variant {
            Variant::Deletion(_)  => self.deletion_factor.ln(),
            Variant::Insertion(_) => self.insertion_factor.ln(),
            Variant::SNV(_) => 0.0 // no factor for SNVs
        };

        LogProb(self.effective_mutation_rate.ln() + factor - (2.0 * af_somatic.ln() + (self.genome_size as f64).ln()))
    }
}


impl PairModel<ContinousAlleleFreqs, DiscreteAlleleFreqs> for TumorNormalModel {

    fn prior_prob(&self, af_tumor: AlleleFreq, af_normal: AlleleFreq, variant: Variant) -> LogProb {
        let p = self.somatic_prior_prob(af_tumor, af_normal, variant) +
                self.normal_model.prior_prob(af_normal, variant);
        assert!(*p <= 0.0);
        p
    }

    fn joint_prob<L, O>(
        &self,
        af_tumor: &ContinousAlleleFreqs,
        af_normal: &DiscreteAlleleFreqs,
        likelihood_tumor: &L,
        likelihood_normal: &O,
        variant: Variant
    ) -> LogProb where
        L: Fn(AlleleFreq, AlleleFreq) -> LogProb,
        O: Fn(AlleleFreq, AlleleFreq) -> LogProb
    {
        /*fn qng_density<D: Fn(f64) -> LogProb>(af: f64, density: &mut D) -> f64 {
            density(af).exp()
        }*/

        let prob = LogProb::ln_sum_exp(&af_normal.iter().map(|&af_normal| {
            let density = |af_tumor| {
                let af_tumor = AlleleFreq(af_tumor);
                self.prior_prob(af_tumor, af_normal, variant) +
                likelihood_tumor(af_tumor, af_normal)
            };

            let p_tumor = if af_tumor.start == af_tumor.end {
                density(*af_tumor.start)
            } else {
                LogProb::ln_simpsons_integrate_exp(&density, *af_tumor.start, *af_tumor.end, self.grid_points)
                //let p = LogProb::ln_trapezoidal_integrate_exp(&density, *af_tumor.start, *af_tumor.end, self.grid_points);
                /*let mut res = 0.0;
                let mut err = 0.0;
                let mut n = 0;
                qng(qng_density, &mut density, *af_tumor.start, *af_tumor.end, 0.5, 0.5, &mut res, &mut err, &mut n);
                println!("{}=?{}=?{} {} {}", p.exp(), res, p2.exp(), err, n);
                //p
                LogProb(res.ln())*/
            };
            let p_normal = likelihood_normal(af_normal, AlleleFreq(0.0));
            let prob = p_tumor + p_normal;

            prob
        }).collect_vec());

        prob
    }

    fn marginal_prob<L, O>(
        &self,
        likelihood_tumor: &L,
        likelihood_normal: &O,
        variant: Variant
    ) -> LogProb where
        L: Fn(AlleleFreq, AlleleFreq) -> LogProb,
        O: Fn(AlleleFreq, AlleleFreq) -> LogProb
    {
        let p = self.joint_prob(
            self.allele_freqs_case(),
            self.allele_freqs_control(),
            likelihood_tumor,
            likelihood_normal,
            variant
        ).ln_add_exp(
            // add prob for allele frequency zero (the density is non-continuous there)
            self.joint_prob(
                &(AlleleFreq(0.0)..AlleleFreq(0.0)),
                &vec![AlleleFreq(0.0)],
                likelihood_tumor,
                likelihood_normal,
                variant
            )
        );
        p
    }

    fn map<L, O>(
        &self,
        likelihood_tumor: &L,
        likelihood_normal: &O,
        variant: Variant
    ) -> (AlleleFreq, AlleleFreq) where
        L: Fn(AlleleFreq, AlleleFreq) -> LogProb,
        O: Fn(AlleleFreq, AlleleFreq) -> LogProb
    {
        let af_case = linspace(*self.allele_freqs_tumor.start, *self.allele_freqs_tumor.end, self.grid_points);
        let (_, (map_normal, map_tumor)) = self.normal_model.allele_freqs().iter().cartesian_product(af_case).minmax_by_key(
            |&(&af_normal, af_tumor)| {
                let af_tumor = AlleleFreq(af_tumor);
                let p = self.prior_prob(af_tumor, af_normal, variant) +
                        likelihood_tumor(af_tumor, af_normal) +
                        likelihood_normal(af_normal, AlleleFreq(0.0));
                //println!("af {} vs {} = {} (prior={} tumor={} normal={})", *af_tumor, af_normal, *p, *self.prior_prob(af_tumor, af_normal, variant), *likelihood_tumor(af_tumor, af_normal), *likelihood_normal(af_normal, AlleleFreq(0.0)));
                NotNaN::new(*p).expect("posterior probability is NaN")
            }
        ).into_option().expect("prior has empty allele frequency spectrum");

        (AlleleFreq(map_tumor), *map_normal)
    }

    fn allele_freqs_case(&self) -> &ContinousAlleleFreqs {
        &self.allele_freqs_tumor
    }

    fn allele_freqs_control(&self) -> &DiscreteAlleleFreqs {
        self.normal_model.allele_freqs()
    }
}