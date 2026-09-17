//! Base fragments: reusable graph-building blocks that reproduce the node
//! shapes assembled inline in `pipeline::{checkpoint_txt2img, flux_txt2img,
//! flux2_klein_txt2img, flux2_klein_txt2img_safetensors}`. A later task wires
//! recipes to build from these instead of inline `json!` graphs; this task
//! only proves the fragments reproduce today's output byte-for-byte (see
//! `cross_check` below).

pub mod conditioning;
pub mod latent;
pub mod loaders;
pub mod output;
pub mod sampling;

pub use conditioning::Cond;
pub use loaders::{FluxGgufIds, Loaded};
pub use sampling::{CustomAdvancedIds, CustomAdvancedParams, SamplerParams};

#[cfg(test)]
mod cross_check {
    use crate::pipeline::graph::Graph;
    use crate::pipeline::{self, Txt2ImgInputs};

    use super::{conditioning, latent, loaders, output, sampling};

    fn inputs() -> Txt2ImgInputs<'static> {
        Txt2ImgInputs {
            positive: "a red fox in the snow",
            negative: "blurry, low quality",
            width: 1024,
            height: 1024,
            steps: 25,
            cfg: 7.0,
            sampler: "euler",
            scheduler: "normal",
            seed: 42,
            filename_prefix: "job-abc",
        }
    }

    /// Proves the fragments reproduce `checkpoint_txt2img`'s no-LoRA graph
    /// byte-identically -- the point of the fragment layer, and what a later
    /// task will rely on when it ports the recipe to build from fragments.
    #[test]
    fn checkpoint_txt2img_fragments_match_the_existing_recipe() {
        let i = inputs();
        let expected = pipeline::checkpoint_txt2img(&i, "sd_xl_base_1.0.safetensors", &[]);

        let mut g = Graph::default();
        let loaded = loaders::checkpoint(&mut g, "4", "sd_xl_base_1.0.safetensors");
        let latent = latent::empty(&mut g, "5", i.width, i.height);
        let cond =
            conditioning::encode_pair(&mut g, "6", "7", &loaded.clip, i.positive, i.negative);
        let sampled = sampling::ksampler(
            &mut g,
            "3",
            &loaded.model,
            &cond.positive,
            &cond.negative,
            &latent,
            &sampling::SamplerParams {
                seed: i.seed,
                steps: i.steps,
                cfg: i.cfg,
                sampler: i.sampler,
                scheduler: i.scheduler,
                denoise: 1.0,
            },
        );
        output::decode_and_save(&mut g, "8", "9", &sampled, &loaded.vae, i.filename_prefix);

        assert_eq!(g.into_value(), expected);
    }
}
