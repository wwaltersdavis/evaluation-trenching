mod intersects;
mod io;
mod run;
mod statistics;
mod trench;

use anyhow::{bail, Context, Result};
use evaluation::{Distribution, Percentage, TrenchConfig};
fn parse_placement_settings() -> Result<trench::PlacementSettings> {
    let mut settings = trench::PlacementSettings::default();
    let mut arguments = std::env::args().skip(1);

    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--layouts" => {
                let value = arguments
                    .next()
                    .context("Expected a number after --layouts")?;

                settings.number_of_layouts_per_area =
                    value.parse::<usize>().with_context(|| {
                        format!("Could not read `{value}` as the number of layouts")
                    })?;

                if settings.number_of_layouts_per_area == 0 {
                    bail!("--layouts must be greater than zero");
                }
            }
            "--seed" => {
                let value = arguments.next().context("Expected a number after --seed")?;

                settings.seed = value
                    .parse::<u64>()
                    .with_context(|| format!("Could not read `{value}` as the random seed"))?;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: evaluation [--layouts NUMBER] [--seed NUMBER]\n\
                     \n\
                     --layouts NUMBER  Placements attempted for each LOE (default: 180)\n\
                     --seed NUMBER     Reproducible random seed (default: 20261124)"
                );
                std::process::exit(0);
            }
            _ => {
                bail!("Unknown argument `{argument}`. Use --help to see the available arguments.");
            }
        }
    }

    Ok(settings)
}

fn main() -> Result<()> {
    let selected_layer: Option<&str> = None;
    let placement_settings = parse_placement_settings()?;

    println!(
        "Placement settings: {} layouts per LOE, seed {}",
        placement_settings.number_of_layouts_per_area, placement_settings.seed
    );

    let coverage_percentages = [1.0, 2.0, 3.0, 4.0, 5.0, 10.0, 15.0, 20.0];

    for coverage in coverage_percentages {
        let continuous =
            TrenchConfig::continuous(2.0, Distribution::Coverage(Percentage::from(coverage)));

        let parallel_array = TrenchConfig::parallel_array(
            2.0,
            30.0,
            Distribution::Coverage(Percentage::from(coverage)),
        );

        let standard_grid = TrenchConfig::standard_grid(
            2.0,
            30.0,
            Distribution::Coverage(Percentage::from(coverage)),
        );

        let test_pits =
            TrenchConfig::test_pits(1.0, Distribution::Coverage(Percentage::from(coverage)));

        for config in [continuous, parallel_array, standard_grid, test_pits].iter() {
            run::all_loes(config, selected_layer, placement_settings)?;
        }
    }
    Ok(())
}
