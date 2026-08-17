// Folder production workflow: attach every LAS/LAZ tile, report per-class
// statistics, reclassify an elevation band as ground, and write one merged
// delivery file. Edit the constants below and run with:
//   SCRIPT scripts/library/folder_workflow.rhai

let folder = "D:\\MA_Lidar\\Boston_Central_2013_2014_usgs_post_sandy_ma_nh_ri_J1434573";
let export_path = "D:\\MA_Lidar\\merged_delivery.laz";
let band_low = 0.0;      // survey feet, Z band treated as bare ground
let band_high = 12.0;
let ground_class = 2;

ocs.log("attaching folder: " + folder);
ocs.cloud_attach_folder(folder);

let sources = ocs.cloud_sources();
ocs.log("sources attached: " + sources.len());

let stats = ocs.cloud_stats();
for class in stats.keys() {
    ocs.log("class " + class + ": " + stats[class] + " points");
}

ocs.cloud_select_slice(band_low, band_high);
let changed = ocs.cloud_classify_selection(ground_class);
ocs.log("reclassified " + changed + " band points as class " + ground_class);
ocs.cloud_select_clear();

ocs.log("exporting merged cloud to " + export_path);
ocs.cloud_export_all(export_path);

// Wait for the merged export to finish so the script ends only when the
// delivery file is complete.
let guard = 0;
loop {
    let status = ocs.cloud_export_status();
    if !status.running {
        break;
    }
    guard = guard + 1;
    if guard > 12000 {
        ocs.log("export still running; script exiting — check POINTCLOUDEXPORTSTATUS");
        break;
    }
}
ocs.log("workflow complete");
