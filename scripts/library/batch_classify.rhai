// Batch classification over a folder of LAS/LAZ tiles: for each file —
// attach, remove isolated noise, densify bare-earth ground, optionally fuse
// official building/road/tree reference layers (urban UPCP labels), export a
// classified copy, detach, and move to the next. Edit the constants below
// and run with:
//   SCRIPT scripts/library/batch_classify.rhai

let folder = "D:\\MA_Lidar\\Boston_Central_2013_2014_usgs_post_sandy_ma_nh_ri_J1434573";
let noise_radius = 2.0;        // project units
let noise_neighbors = 4;       // fewer neighbors than this inside radius = noise
let noise_class = 7;
let ground_cell = 10.0;        // surface grid cell (project units)
let ground_distance = 0.75;    // max distance from local surface
let ground_angle = 30.0;       // max slope angle in degrees
// Urban fusion adds a UPCP `label` extra byte from official reference
// layers; the source ASPRS classification byte is never modified. Leave
// false for purely geometric batches.
let urban_fusion = false;

let files = ocs.cloud_list_folder(folder);
if files.len() == 0 {
    ocs.log("no LAS/LAZ files in " + folder);
} else {
    ocs.log("batch: " + files.len() + " file(s)");
    let done = 0;
    for file in files {
        ocs.log("--- " + file);

        ocs.cloud_attach(file);
        // Wait for the bounded sample to land so the classifiers see data.
        let wait = 0;
        while ocs.cloud_sources().len() == 0 && wait < 1200 {
            wait = wait + 1;
        }
        if ocs.cloud_sources().len() == 0 {
            ocs.log("attach did not complete; skipping file");
            ocs.cloud_detach();
            continue;
        }

        ocs.command("POINTCLOUDNOISE " + noise_radius + " " + noise_neighbors + " " + noise_class);
        ocs.command("POINTCLOUDGROUND " + ground_cell + " " + ground_distance + " " + ground_angle);

        if urban_fusion {
            // Full-density native fusion: seeds from the ASPRS classes the
            // geometric stages produced, then fuses buildings/roads/trees.
            let started = ocs.cloud_urban_classify(`{"scope":"current"}`);
            if !started.started {
                ocs.log("urban fusion refused: " + started.reason);
            } else {
                let guard = 0;
                loop {
                    let status = ocs.cloud_urban_status();
                    if !status.running {
                        if status.status != "" {
                            ocs.log("urban fusion: " + status.status);
                        }
                        break;
                    }
                    guard = guard + 1;
                    if guard > 30000 {
                        ocs.log("urban fusion stalled; continuing");
                        break;
                    }
                }
            }
        }

        // Both ".las" and ".laz" are four characters: trim the extension
        // and append the delivery suffix.
        let stem = file.sub_string(0, file.len() - 4);
        let out = stem + "_classified.laz";
        ocs.cloud_export_all(out);
        let guard = 0;
        loop {
            let status = ocs.cloud_export_status();
            if !status.running {
                break;
            }
            guard = guard + 1;
            if guard > 30000 {
                ocs.log("export stalled; continuing");
                break;
            }
        }

        ocs.cloud_detach();
        done = done + 1;
        ocs.log("done: " + out);
    }
    ocs.log("batch complete: " + done + "/" + files.len() + " file(s)");
}
