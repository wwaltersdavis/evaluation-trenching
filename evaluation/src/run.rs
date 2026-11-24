use crate::{
    intersects, io,
    statistics::{
        write_eligibility_to_csv, write_layouts_to_csv, write_to_csv, EligibilityRow, LayoutRow,
        StatsMap,
    },
    trench,
};
use anyhow::Result;
use rayon::prelude::*;
use std::time::Instant;

use evaluation::TrenchConfig;
fn derive_loe_seed(base_seed: u64, site: &str, loe_id: &str) -> u64 {
    let mut derived_seed = 0xcbf29ce484222325_u64 ^ base_seed;

    for byte in site
        .bytes()
        .chain(std::iter::once(b'/'))
        .chain(loe_id.bytes())
    {
        derived_seed ^= byte as u64;
        derived_seed = derived_seed.wrapping_mul(0x100000001b3);
    }

    derived_seed
}
pub fn all_loes(
    config: &TrenchConfig,
    selected_layer: Option<&str>,
    placement_settings: trench::PlacementSettings,
) -> Result<()> {
    println!("\nRunning {:?} on all LOEs", config.pattern_name);
    let test_locations = io::read_all_test_location_data(selected_layer)?;

    let now = Instant::now();

    let mut total_trench_creation_time = 0;
    let mut total_testing_time = 0;

    let mut all_stats_map: StatsMap = StatsMap::new();
    let mut layout_rows: Vec<LayoutRow> = Vec::new();
    let mut eligibility_rows: Vec<EligibilityRow> = Vec::new();

    for test_location in &test_locations {
        let loe_settings = trench::PlacementSettings {
            number_of_layouts_per_area: placement_settings.number_of_layouts_per_area,
            seed: derive_loe_seed(
                placement_settings.seed,
                test_location.site_name.as_str(),
                test_location.loe_id.as_str(),
            ),
        };

        println!(
            "LOE {}/{} placement seed: {}",
            test_location.site_name, test_location.loe_id, loe_settings.seed
        );

        let trenches_time = Instant::now();
        let trenches = trench::create_layouts(
            config,
            test_location.limit_of_excavation.clone(),
            loe_settings,
        );
        total_trench_creation_time += trenches_time.elapsed().as_millis();

        let retained_layouts = trenches.as_ref().map_or(0, Vec::len);

        eligibility_rows.push(EligibilityRow::new(
            test_location.site_name.as_str(),
            test_location.loe_id.as_str(),
            config,
            loe_settings.seed,
            loe_settings.number_of_layouts_per_area,
            retained_layouts,
        ));

        let testing_time = Instant::now();

        match trenches {
            Some(trenches) => {
                let layout_results: Vec<_> = trenches
                    .into_par_iter()
                    .map(|trench| {
                        let stats_map = intersects::test_features_for_layout(
                            &test_location.features,
                            &trench,
                            test_location.site_name.as_str(),
                            test_location.loe_id.as_str(),
                        );

                        (trench, stats_map)
                    })
                    .collect();

                let mut stats_maps = Vec::with_capacity(layout_results.len());

                for (trench, stats_map) in layout_results {
                    let overall_stats = stats_map
                        .0
                        .get(&("All", "All", "All", "All"))
                        .expect("Overall layout statistics should exist");

                    layout_rows.push(LayoutRow::from_stats(
                        test_location.site_name.as_str(),
                        test_location.loe_id.as_str(),
                        config,
                        &config.distribution,
                        loe_settings.seed,
                        loe_settings.number_of_layouts_per_area,
                        &trench,
                        overall_stats,
                    ));

                    stats_maps.push(stats_map);
                }

                all_stats_map.add_from_maps(&stats_maps);
            }
            None => {
                println!("No trenches created for LOE");
                continue;
            }
        }

        total_testing_time += testing_time.elapsed().as_millis();
    }

    println!(
        "Creating trenches took: {:?}s",
        total_trench_creation_time as f64 / 1000.0
    );
    println!(
        "Testing trenches took: {:?}s",
        total_testing_time as f64 / 1000.0
    );

    println!("{}", all_stats_map.general_stats_message());

    let output_path = format!(
        "../data/results_{:?}_{}.csv",
        config.pattern_name,
        config.distribution.to_file_string()
    );

    write_to_csv(
        output_path.as_str(),
        config,
        &config.distribution,
        all_stats_map,
    )?;

    println!("Saved CSV: {}", output_path);
    let layout_output_path = format!(
        "../data/results_{:?}_{}_layouts.csv",
        config.pattern_name,
        config.distribution.to_file_string()
    );

    write_layouts_to_csv(layout_output_path.as_str(), &layout_rows)?;

    println!("Saved layout CSV: {}", layout_output_path);

    let eligibility_output_path = format!(
        "../data/results_{:?}_{}_eligibility.csv",
        config.pattern_name,
        config.distribution.to_file_string()
    );

    write_eligibility_to_csv(eligibility_output_path.as_str(), &eligibility_rows)?;

    println!("Saved eligibility CSV: {}", eligibility_output_path);
    println!("Testing took: {:?}", now.elapsed());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loe_seed_is_reproducible_and_location_specific() {
        let first = derive_loe_seed(20261124, "Heathrow", "0");
        let repeated = derive_loe_seed(20261124, "Heathrow", "0");
        let different_loe = derive_loe_seed(20261124, "Heathrow", "1");
        let different_site = derive_loe_seed(20261124, "Stansted", "0");

        assert_eq!(first, repeated);
        assert_ne!(first, different_loe);
        assert_ne!(first, different_site);
    }
}
