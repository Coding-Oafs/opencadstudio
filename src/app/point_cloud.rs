//! Native LAS/LAZ attachment and classification workflow.
//!
//! A tab owns a bounded display sample and sparse edits. The original source
//! remains authoritative until the user explicitly exports a new file.

use super::{Message, OpenCADStudio};
use crate::scene::WireModel;
use iced::Task;
use ocs_pointcloud::{ClassificationEdits, ExportStats, PointSample, SampleOptions};
use std::{collections::BTreeMap, path::PathBuf};

const DISPLAY_POINT_LIMIT: usize = 50_000;
const DISPLAY_READ_CHUNK: usize = 65_536;
const MAX_COMMAND_EDIT_POINTS: usize = 5_000_000;

#[derive(Clone, Debug)]
pub(super) struct PointCloudAttachment {
    pub(super) source_path: PathBuf,
    pub(super) sample: PointSample,
    pub(super) edits: ClassificationEdits,
}

impl PointCloudAttachment {
    pub(super) fn new(source_path: PathBuf, sample: PointSample) -> Self {
        Self {
            source_path,
            sample,
            edits: ClassificationEdits::default(),
        }
    }

    pub(super) fn display_wires(&self) -> Vec<WireModel> {
        let mut classes: BTreeMap<u8, Vec<[f64; 3]>> = BTreeMap::new();
        let bounds = &self.sample.metadata;
        let span = (bounds.bounds_max[0] - bounds.bounds_min[0])
            .abs()
            .max((bounds.bounds_max[1] - bounds.bounds_min[1]).abs())
            .max((bounds.bounds_max[2] - bounds.bounds_min[2]).abs());
        let half = (span / 4_000.0).clamp(0.01, 10.0);

        for point in &self.sample.points {
            let classification = self
                .edits
                .classification_for(point.source_index)
                .unwrap_or(point.classification);
            let [x, y, z] = point.position;
            let vertices = classes.entry(classification).or_default();
            vertices.extend_from_slice(&[
                [x - half, y, z],
                [x + half, y, z],
                [f64::NAN; 3],
                [x, y - half, z],
                [x, y + half, z],
                [f64::NAN; 3],
            ]);
        }

        classes
            .into_iter()
            .map(|(classification, points)| {
                WireModel::solid_f64(
                    format!("POINTCLOUD_CLASS_{classification}"),
                    points,
                    classification_color(classification),
                    false,
                )
            })
            .collect()
    }

    pub(super) fn suggested_export_name(&self) -> String {
        let stem = self
            .source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("point-cloud");
        let extension = self
            .source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| {
                extension.eq_ignore_ascii_case("las") || extension.eq_ignore_ascii_case("laz")
            })
            .unwrap_or("laz");
        format!("{stem}_classified.{extension}")
    }
}

impl OpenCADStudio {
    pub(super) fn start_point_cloud_load(&mut self, path: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        self.command_line.push_info(
            format!(
                "POINTCLOUDATTACH: reading bounded display sample from \"{}\"...",
                path.display()
            )
            .as_str(),
        );
        let worker_path = path.clone();
        background_task(
            move || {
                ocs_pointcloud::sample(
                    &worker_path,
                    SampleOptions {
                        max_points: DISPLAY_POINT_LIMIT,
                        chunk_size: DISPLAY_READ_CHUNK,
                    },
                )
                .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudLoaded(tab_id, path, result),
        )
    }

    pub(super) fn install_point_cloud(
        &mut self,
        tab_id: u64,
        path: PathBuf,
        result: Result<PointSample, String>,
    ) -> Task<Message> {
        let Some(tab_index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            self.command_line
                .push_info("POINTCLOUDATTACH: target drawing was closed.");
            return Task::none();
        };
        let sample = match result {
            Ok(sample) => sample,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDATTACH: {error}").as_str());
                return Task::none();
            }
        };

        let attachment = PointCloudAttachment::new(path.clone(), sample);
        let metadata = &attachment.sample.metadata;
        let point_count = metadata.point_count;
        let sampled = attachment.sample.points.len();
        let format = metadata.point_format;
        let version = format!("{}.{}", metadata.version_major, metadata.version_minor);
        let compressed = if metadata.compressed { "LAZ" } else { "LAS" };
        let bounds_min = metadata.bounds_min;
        let bounds_max = metadata.bounds_max;
        let wires = attachment.display_wires();
        self.tabs[tab_index].scene.set_point_cloud_wires(wires);
        self.tabs[tab_index].point_cloud = Some(attachment);
        self.tabs[tab_index]
            .scene
            .fit_external_bounds(bounds_min, bounds_max);

        self.command_line.push_output(
            format!(
                "POINTCLOUDATTACH: {} points ({compressed}, LAS {version}, format {format}); displaying {sampled} sampled points. Bounds [{:.3}, {:.3}, {:.3}] to [{:.3}, {:.3}, {:.3}].",
                point_count,
                bounds_min[0],
                bounds_min[1],
                bounds_min[2],
                bounds_max[0],
                bounds_max[1],
                bounds_max[2],
            )
            .as_str(),
        );
        Task::none()
    }

    pub(super) fn point_cloud_info(&mut self, tab_index: usize) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_ref() else {
            self.command_line
                .push_info("POINTCLOUDINFO: no LAS/LAZ cloud is attached.");
            return;
        };
        let metadata = &cloud.sample.metadata;
        self.command_line.push_output(
            format!(
                "POINTCLOUDINFO: \"{}\"; {} source points; {} displayed (stride {}); {} pending classification edits; CRS metadata: {}; VLRs: {}, EVLRs: {}.",
                cloud.source_path.display(),
                metadata.point_count,
                cloud.sample.points.len(),
                cloud.sample.stride,
                cloud.edits.len(),
                if metadata.has_crs { "present" } else { "not declared" },
                metadata.vlr_count,
                metadata.evlr_count,
            )
            .as_str(),
        );
    }

    pub(super) fn reclassify_point_cloud(
        &mut self,
        tab_index: usize,
        classification: u8,
        index_spec: &str,
    ) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_error("POINTCLOUDCLASSIFY: attach a LAS/LAZ cloud first.");
            return;
        };
        let indices = match parse_source_indices(index_spec, cloud.sample.metadata.point_count) {
            Ok(indices) => indices,
            Err(error) => {
                self.command_line
                    .push_error(format!("POINTCLOUDCLASSIFY: {error}").as_str());
                return;
            }
        };
        let changed = cloud.edits.reclassify(indices, classification);
        let wires = cloud.display_wires();
        self.tabs[tab_index].scene.set_point_cloud_wires(wires);
        self.command_line.push_output(
            format!(
                "POINTCLOUDCLASSIFY: queued {changed} point(s) as class {classification}; export to create a revised LAS/LAZ."
            )
            .as_str(),
        );
    }

    pub(super) fn undo_point_cloud_edit(&mut self, tab_index: usize) {
        let Some(cloud) = self.tabs[tab_index].point_cloud.as_mut() else {
            self.command_line
                .push_info("POINTCLOUDUNDO: no LAS/LAZ cloud is attached.");
            return;
        };
        if cloud.edits.undo() {
            let wires = cloud.display_wires();
            self.tabs[tab_index].scene.set_point_cloud_wires(wires);
            self.command_line
                .push_output("POINTCLOUDUNDO: restored the previous classification edit state.");
        } else {
            self.command_line
                .push_info("POINTCLOUDUNDO: no point-cloud edit to undo.");
        }
    }

    pub(super) fn detach_point_cloud(&mut self, tab_index: usize) {
        if self.tabs[tab_index].point_cloud.take().is_some() {
            self.tabs[tab_index].scene.set_point_cloud_wires(Vec::new());
            self.command_line.push_output(
                "POINTCLOUDDETACH: detached the session cloud; the source file was unchanged.",
            );
        } else {
            self.command_line
                .push_info("POINTCLOUDDETACH: no LAS/LAZ cloud is attached.");
        }
    }

    pub(super) fn start_point_cloud_export(&mut self, output: PathBuf) -> Task<Message> {
        let tab_id = self.tabs[self.active_tab].id;
        let Some(cloud) = self.tabs[self.active_tab].point_cloud.as_ref() else {
            self.command_line
                .push_error("POINTCLOUDEXPORT: attach a LAS/LAZ cloud first.");
            return Task::none();
        };
        let input = cloud.source_path.clone();
        let edits = cloud.edits.clone();
        let worker_output = output.clone();
        self.command_line.push_info(
            format!(
                "POINTCLOUDEXPORT: streaming {} source points to \"{}\"...",
                cloud.sample.metadata.point_count,
                output.display()
            )
            .as_str(),
        );
        background_task(
            move || {
                ocs_pointcloud::export_with_edits(input, &worker_output, &edits)
                    .map_err(|error| error.to_string())
            },
            move |result| Message::PointCloudExported(tab_id, output, result),
        )
    }

    pub(super) fn finish_point_cloud_export(
        &mut self,
        tab_id: u64,
        output: PathBuf,
        result: Result<ExportStats, String>,
    ) {
        if !self.tabs.iter().any(|tab| tab.id == tab_id) {
            self.command_line
                .push_info("POINTCLOUDEXPORT: target drawing was closed; export result follows.");
        }
        match result {
            Ok(stats) => self.command_line.push_output(
                format!(
                    "POINTCLOUDEXPORT: wrote {} points to \"{}\"; {} classification edits applied.",
                    stats.points_written,
                    output.display(),
                    stats.points_reclassified,
                )
                .as_str(),
            ),
            Err(error) => self
                .command_line
                .push_error(format!("POINTCLOUDEXPORT: {error}").as_str()),
        }
    }
}

fn background_task<T, F, M>(work: F, map: M) -> Task<Message>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    M: FnOnce(T) -> Message + Send + 'static,
{
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    Task::perform(
        async move { receiver.await.expect("point-cloud worker dropped") },
        map,
    )
}

fn classification_color(classification: u8) -> [f32; 4] {
    match classification {
        0 => [0.55, 0.55, 0.55, 1.0],
        1 => [0.82, 0.82, 0.82, 1.0],
        2 => [0.64, 0.42, 0.22, 1.0],
        3 => [0.45, 0.78, 0.34, 1.0],
        4 => [0.20, 0.65, 0.22, 1.0],
        5 => [0.05, 0.42, 0.10, 1.0],
        6 => [0.90, 0.22, 0.18, 1.0],
        7 | 18 => [0.86, 0.20, 0.78, 1.0],
        9 => [0.16, 0.48, 0.95, 1.0],
        12 => [1.00, 0.78, 0.12, 1.0],
        17 => [0.15, 0.85, 0.95, 1.0],
        _ => [0.92, 0.92, 0.92, 1.0],
    }
}

fn parse_source_indices(spec: &str, point_count: u64) -> Result<Vec<u64>, String> {
    let mut indices = Vec::new();
    for token in spec
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        if let Some((start, end)) = token.split_once('-') {
            let start = start
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid source index range: {token}"))?;
            let end = end
                .trim()
                .parse::<u64>()
                .map_err(|_| format!("invalid source index range: {token}"))?;
            if start > end {
                return Err(format!("range starts after it ends: {token}"));
            }
            let count = usize::try_from(end - start + 1).unwrap_or(usize::MAX);
            if indices.len().saturating_add(count) > MAX_COMMAND_EDIT_POINTS {
                return Err(format!(
                    "one command is limited to {MAX_COMMAND_EDIT_POINTS} point indices"
                ));
            }
            indices.extend(start..=end);
        } else {
            indices.push(
                token
                    .parse::<u64>()
                    .map_err(|_| format!("invalid source index: {token}"))?,
            );
        }
    }
    if indices.is_empty() {
        return Err("provide source indices such as 10,25-40".into());
    }
    if let Some(index) = indices.iter().copied().find(|&index| index >= point_count) {
        return Err(format!(
            "source index {index} is outside this cloud (0..{})",
            point_count.saturating_sub(1)
        ));
    }
    Ok(indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_individual_indices_and_inclusive_ranges() {
        assert_eq!(
            vec![1, 3, 4, 5, 8],
            parse_source_indices("1,3-5,8", 10).unwrap()
        );
    }

    #[test]
    fn rejects_reversed_and_out_of_bounds_ranges() {
        assert!(parse_source_indices("5-3", 10).is_err());
        assert!(parse_source_indices("9-10", 10).is_err());
    }
}
