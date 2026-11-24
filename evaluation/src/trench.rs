use evaluation::array::{Configuration, PatternRotationAxis};
use evaluation::{
    Degree, Distribution, Percentage, Rectangle, Structure, TrenchConfig, TrenchLayout,
};
use geo::{
    coord, Area, BooleanOps, Centroid, Distance, Euclidean, Intersects, LineString, MultiPolygon,
    Point, Polygon, Rotate, Translate,
};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use std::panic;

#[derive(Debug, Clone, Copy)]
pub struct PlacementSettings {
    pub number_of_layouts_per_area: usize,
    pub seed: u64,
}

impl Default for PlacementSettings {
    fn default() -> Self {
        Self {
            number_of_layouts_per_area: 180,
            seed: 20261124,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Placement {
    angle_degrees: f64,
    x_phase: f64,
    y_phase: f64,
}
fn generate_placements(
    structure: Structure,
    number_of_layouts: usize,
    seed: u64,
) -> Vec<Placement> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let maximum_angle = structure.get_rotational_symmetry() as f64;

    (0..number_of_layouts)
        .map(|_| Placement {
            angle_degrees: rng.random_range(0.0..maximum_angle),
            x_phase: rng.random_range(0.0..1.0),
            y_phase: rng.random_range(0.0..1.0),
        })
        .collect()
}
pub fn create_layouts(
    config: &TrenchConfig,
    limit_of_excavation: Polygon,
    placement_settings: PlacementSettings,
) -> Option<Vec<TrenchLayout>> {
    let centroid = limit_of_excavation.centroid().unwrap();
    let max_distance_from_centroid = get_max_distance_from_centroid(centroid, &limit_of_excavation);

    match config.distribution {
        Distribution::Spacing(spacing) => Some(get_layouts_from_spacing(
            &limit_of_excavation,
            *config,
            max_distance_from_centroid,
            centroid,
            spacing,
        )),
        Distribution::Coverage(coverage) => get_layouts_from_coverage(
            &limit_of_excavation,
            *config,
            max_distance_from_centroid,
            centroid,
            coverage,
            placement_settings,
        ),
    }
}

fn get_size_of_grid(max_distance_from_centroid: &f64, spacing: &f64) -> i32 {
    (max_distance_from_centroid / spacing).ceil() as i32 + 2
}

fn spacing_smaller_than_minimum(spacing: &f64, minimum_spacing: &f64) -> bool {
    spacing < minimum_spacing
}

fn estimate_spacing(config: &TrenchConfig, coverage: &Percentage) -> f64 {
    match config.structure {
        Structure::Parallel(line) => line.width / (coverage.percentage_as_decimal()),
        Structure::Array(rectangle, array_config) => {
            if array_config.separated {
                let y_spacing = rectangle.length + 1.0;
                (rectangle.width * rectangle.length)
                    / (coverage.percentage_as_decimal() * y_spacing * 2.0)
            } else {
                ((rectangle.width * rectangle.length) / (coverage.percentage_as_decimal())).sqrt()
            }
        }
    }
}

fn trench_of_array_coordinate(
    x_index: usize,
    y_index: usize,
    x_offset: i32,
    y_offset: i32,
    centroid: Point,
    spacing: f64,
    array_config: &Configuration,
    rectangle: Rectangle,
) -> Option<Polygon> {
    let y_spacing = if array_config.separated {
        rectangle.length + 1.0
    } else {
        spacing
    };

    let trench_centroid =
        centroid.translate(x_offset as f64 * spacing, y_offset as f64 * y_spacing);
    let is_alternate_point = (x_index + y_index).is_multiple_of(2);
    let rotation = match array_config.pattern_rotation_axis {
        PatternRotationAxis::ByCell => {
            if is_alternate_point {
                array_config.base_angle
            } else {
                array_config.alternate_angle
            }
        }
        PatternRotationAxis::ByColumn => {
            if x_index.is_multiple_of(2) {
                array_config.base_angle
            } else {
                array_config.alternate_angle
            }
        }
    };

    if array_config.separated && is_alternate_point {
        None
    } else {
        Some(plot_trench(
            trench_centroid,
            rectangle.width,
            rectangle.length,
            rotation,
        ))
    }
}

fn plot_trench(centroid: Point, width: f64, length: f64, rotation: Degree) -> Polygon<f64> {
    let trench_exterior = vec![
        coord! { x: centroid.x() - width / 2.0, y: centroid.y() - length / 2.0 },
        coord! { x: centroid.x() + width / 2.0, y: centroid.y() - length / 2.0 },
        coord! { x: centroid.x() + width / 2.0, y: centroid.y() + length / 2.0 },
        coord! { x: centroid.x() - width / 2.0, y: centroid.y() + length / 2.0 },
        coord! { x: centroid.x() - width / 2.0, y: centroid.y() - length / 2.0 },
    ];
    Polygon::new(LineString(trench_exterior), vec![]).rotate_around_point(rotation.0, centroid)
}

fn get_layouts_from_coverage(
    limit_of_excavation: &Polygon,
    config: TrenchConfig,
    max_distance_from_centroid: f64,
    centroid: Point,
    coverage: Percentage,
    placement_settings: PlacementSettings,
) -> Option<Vec<TrenchLayout>> {
    let estimated_spacing = estimate_spacing(&config, &coverage);
    if spacing_smaller_than_minimum(&estimated_spacing, &config.minimum_spacing) {
        return None;
    }
    let placements = generate_placements(
        config.structure,
        placement_settings.number_of_layouts_per_area,
        placement_settings.seed,
    );

    let limit_of_excavation = MultiPolygon(vec![limit_of_excavation.clone()]);

    let trench_patterns: Vec<TrenchLayout> = placements
        .into_par_iter()
        .enumerate()
        .filter_map(|(layout_index, placement)| {
            adjust_trench_layout_to_coverage(
                coverage.0,
                centroid,
                &limit_of_excavation,
                estimated_spacing,
                &config,
                placement,
                layout_index,
                &max_distance_from_centroid,
            )
        })
        .collect();
    if trench_patterns.is_empty() {
        return None;
    }
    Some(trench_patterns)
}
fn get_positioned_layout(
    config: TrenchConfig,
    max_distance_from_centroid: f64,
    centroid: Point,
    spacing: f64,
    placement: Placement,
) -> MultiPolygon {
    let (x_shift, y_shift) = match config.structure {
        Structure::Parallel(_) => (placement.x_phase * spacing, 0.0),
        Structure::Array(rectangle, array_config) => {
            let y_spacing = if array_config.separated {
                rectangle.length + 1.0
            } else {
                spacing
            };

            (placement.x_phase * spacing, placement.y_phase * y_spacing)
        }
    };

    let shifted_origin = centroid.translate(x_shift, y_shift);

    get_layout_from_spacing(config, max_distance_from_centroid, shifted_origin, spacing)
        .rotate_around_point(placement.angle_degrees, centroid)
}

fn get_layouts_from_spacing(
    limit_of_excavation: &Polygon,
    config: TrenchConfig,
    max_distance_from_centroid: f64,
    centroid: Point,
    spacing: f64,
) -> Vec<TrenchLayout> {
    let trenches = get_layout_from_spacing(config, max_distance_from_centroid, centroid, spacing);
    get_rotated_trench_patterns(
        trenches,
        config.structure.get_rotational_symmetry(),
        centroid,
        limit_of_excavation,
    )
}

fn get_layout_from_spacing(
    config: TrenchConfig,
    max_distance_from_centroid: f64,
    centroid: Point,
    spacing: f64,
) -> MultiPolygon {
    let x_count = get_size_of_grid(&max_distance_from_centroid, &spacing);
    let x_offsets = -x_count..x_count + 1;
    let trenches = match config.structure {
        Structure::Parallel(line) => {
            x_offsets
                .into_par_iter()
                .map(|x_offset| {
                    let trench_centroid = centroid.translate(x_offset as f64 * spacing, 0.0);
                    plot_trench(
                        trench_centroid,
                        line.width,
                        max_distance_from_centroid * 2.0,
                        Degree(0.0),
                    )
                })
                .collect()
            // TODO: test performance of this vs .push() to Vec
        }
        Structure::Array(rectangle, array_config) => {
            let y_spacing = if array_config.separated {
                rectangle.length + 1.0
            } else {
                spacing
            };

            let y_count = get_size_of_grid(&max_distance_from_centroid, &y_spacing);
            let y_offsets = -y_count..y_count + 1;
            x_offsets
                .into_par_iter()
                .enumerate()
                .flat_map(|(x_index, x_offset)| {
                    y_offsets
                        .clone()
                        .enumerate()
                        .filter_map(move |(y_index, y_offset)| {
                            trench_of_array_coordinate(
                                x_index,
                                y_index,
                                x_offset,
                                y_offset,
                                centroid,
                                spacing,
                                &array_config,
                                rectangle,
                            )
                        })
                        .collect::<Vec<Polygon>>()
                })
                .collect::<Vec<Polygon>>()
        }
    };
    MultiPolygon(trenches)
}

fn get_max_distance_from_centroid(centroid: Point, limit_of_excavation: &Polygon) -> f64 {
    let max_distance_from_centroid =
        limit_of_excavation
            .exterior()
            .points()
            .fold(0.0, |max_distance_from_centroid, p| {
                let distance = Euclidean.distance(centroid, p);
                if distance > max_distance_from_centroid {
                    distance
                } else {
                    max_distance_from_centroid
                }
            });
    max_distance_from_centroid
}
fn safe_intersection(a: &MultiPolygon<f64>, b: &MultiPolygon<f64>) -> Option<MultiPolygon<f64>> {
    panic::catch_unwind(|| a.boolean_op(b, geo::OpType::Intersection)).ok()
}

fn calculate_coverage(
    trench_layout: &MultiPolygon<f64>,
    limit_of_excavation: &MultiPolygon<f64>,
) -> f64 {
    trench_layout.unsigned_area() / limit_of_excavation.unsigned_area() * 100.0
}

fn get_rotated_trench_patterns(
    trenches: MultiPolygon,
    rotations: i32,
    centroid: Point,
    limit_of_excavation: &Polygon,
) -> Vec<TrenchLayout> {
    let limit_of_excavation_as_multi = MultiPolygon(vec![limit_of_excavation.clone()]);

    (0..rotations)
        .into_par_iter()
        .filter_map(|rotation| {
            let trench_pattern = trenches.rotate_around_point(rotation as f64, centroid);
            let trenches_deployed = trench_pattern
                .iter()
                .filter(|polygon| polygon.intersects(limit_of_excavation))
                .count() as u64;

            // cut trench to site outline
            let Some(intersection) =
                safe_intersection(&limit_of_excavation_as_multi, &trench_pattern)
            else {
                println!(
                    "Skipped rotation {} because site intersection failed",
                    rotation
                );
                return None;
            };

            let achieved_coverage =
                calculate_coverage(&intersection, &limit_of_excavation_as_multi);

            Some(TrenchLayout {
                layout: intersection,
                trenches_deployed,
                layout_index: rotation as usize,
                angle_degrees: rotation as f64,
                x_phase: 0.0,
                y_phase: 0.0,
                achieved_coverage,
            })
        })
        .collect()
    // TODO: return average percentage coverage
}

fn check_coverage(current_coverage: f64, target_coverage: f64) -> bool {
    current_coverage > target_coverage - 0.05 && current_coverage < target_coverage + 0.05
}

fn adjust_trench_layout_to_coverage(
    target_coverage: f64,
    centroid: Point,
    limit_of_excavation: &MultiPolygon,
    estimated_spacing: f64,
    config: &TrenchConfig,
    placement: Placement,
    layout_index: usize,
    max_distance_from_centroid: &f64,
) -> Option<TrenchLayout> {
    let mut iteration = 0;
    let mut current_spacing = estimated_spacing;

    let mut current_trench_pattern = get_positioned_layout(
        *config,
        *max_distance_from_centroid,
        centroid,
        current_spacing,
        placement,
    );

    let Some(mut intersection) = safe_intersection(limit_of_excavation, &current_trench_pattern)
    else {
        println!(
            "No trenches created for {:?}: initial site intersection failed for placement {:?}",
            config.pattern_name, placement
        );
        return None;
    };

    let mut current_coverage = calculate_coverage(&intersection, limit_of_excavation);

    while iteration <= 50 && !check_coverage(current_coverage, target_coverage) {
        let adjustment_factor = 0.94_f64.powf(iteration as f64);
        let error = (target_coverage - current_coverage) / target_coverage;

        if error < 0.0 {
            current_spacing *= 1.0 + -error * adjustment_factor;
        } else {
            current_spacing /= 1.0 + error * adjustment_factor;
        }

        if spacing_smaller_than_minimum(&current_spacing, &config.minimum_spacing) {
            println!(
                "Spacing too small for {:?}: target {:.2} current {:.2}",
                config.pattern_name, target_coverage, current_coverage
            );
            return None;
        }

        current_trench_pattern = get_positioned_layout(
            *config,
            *max_distance_from_centroid,
            centroid,
            current_spacing,
            placement,
        );

        let Some(next_intersection) =
            safe_intersection(limit_of_excavation, &current_trench_pattern)
        else {
            println!(
                "No trenches created for {:?}: site intersection failed for placement {:?}, iteration {}",
                config.pattern_name, placement, iteration
            );
            return None;
        };

        intersection = next_intersection;
        current_coverage = calculate_coverage(&intersection, limit_of_excavation);
        iteration += 1;
    }

    if check_coverage(current_coverage, target_coverage) {
        let trenches_deployed = current_trench_pattern
            .iter()
            .filter(|polygon| polygon.intersects(limit_of_excavation))
            .count() as u64;

        Some(TrenchLayout {
            layout: intersection,
            trenches_deployed,
            layout_index,
            angle_degrees: placement.angle_degrees,
            x_phase: placement.x_phase,
            y_phase: placement.y_phase,
            achieved_coverage: current_coverage,
        })
    } else {
        println!(
            "Failed {:?}: target {:.2} achieved {:.2} after {} iterations for placement {:?}",
            config.pattern_name, target_coverage, current_coverage, iteration, placement
        );
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_generation_is_reproducible() {
        let structure =
            TrenchConfig::continuous(2.0, Distribution::Coverage(Percentage::from(5.0))).structure;

        let first = generate_placements(structure, 180, 20261124);
        let repeated = generate_placements(structure, 180, 20261124);
        let different = generate_placements(structure, 180, 20261125);

        assert_eq!(first, repeated);
        assert_ne!(first, different);
    }
    #[test]
    fn generated_placements_have_valid_ranges() {
        let structure =
            TrenchConfig::continuous(2.0, Distribution::Coverage(Percentage::from(5.0))).structure;

        let placements = generate_placements(structure, 180, 20261124);
        let maximum_angle = structure.get_rotational_symmetry() as f64;

        assert_eq!(placements.len(), 180);

        for placement in placements {
            assert!(placement.angle_degrees >= 0.0);
            assert!(placement.angle_degrees < maximum_angle);
            assert!(placement.x_phase >= 0.0);
            assert!(placement.x_phase < 1.0);
            assert!(placement.y_phase >= 0.0);
            assert!(placement.y_phase < 1.0);
        }
    }
    #[test]
    fn placement_phase_moves_the_layout() {
        let config = TrenchConfig::continuous(2.0, Distribution::Coverage(Percentage::from(5.0)));
        let centroid = Point::new(0.0, 0.0);
        let maximum_distance = 100.0;
        let spacing = 20.0;

        let original_layout = get_layout_from_spacing(config, maximum_distance, centroid, spacing);

        let zero_phase_layout = get_positioned_layout(
            config,
            maximum_distance,
            centroid,
            spacing,
            Placement {
                angle_degrees: 0.0,
                x_phase: 0.0,
                y_phase: 0.0,
            },
        );

        let shifted_layout = get_positioned_layout(
            config,
            maximum_distance,
            centroid,
            spacing,
            Placement {
                angle_degrees: 0.0,
                x_phase: 0.5,
                y_phase: 0.0,
            },
        );

        assert_eq!(original_layout, zero_phase_layout);
        assert_ne!(original_layout, shifted_layout);
    }
    #[test]
    fn y_phase_moves_an_array_layout() {
        let config =
            TrenchConfig::parallel_array(2.0, 30.0, Distribution::Coverage(Percentage::from(5.0)));
        let centroid = Point::new(0.0, 0.0);
        let maximum_distance = 100.0;
        let spacing = 20.0;

        let zero_phase_layout = get_positioned_layout(
            config,
            maximum_distance,
            centroid,
            spacing,
            Placement {
                angle_degrees: 0.0,
                x_phase: 0.0,
                y_phase: 0.0,
            },
        );

        let shifted_layout = get_positioned_layout(
            config,
            maximum_distance,
            centroid,
            spacing,
            Placement {
                angle_degrees: 0.0,
                x_phase: 0.0,
                y_phase: 0.5,
            },
        );

        assert_ne!(zero_phase_layout, shifted_layout);
    }
}
