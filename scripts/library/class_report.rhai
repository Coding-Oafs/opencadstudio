// Per-source class report: prints every attached tile with its point count,
// pending edits, and per-class totals — the daily sanity check before any
// classification work. Run with:
//   SCRIPT scripts/library/class_report.rhai

let sources = ocs.cloud_sources();
if sources.len() == 0 {
    ocs.log("no LAS/LAZ sources attached; run POINTCLOUDATTACHFOLDER first");
} else {
    for source in sources {
        ocs.log(
            source.id + ": " + source.points + " points (" +
            source.displayed + " displayed), " + source.edits + " pending edits — " +
            source.path
        );
    }
    let stats = ocs.cloud_stats();
    ocs.log("--- dataset classes ---");
    for class in stats.keys() {
        ocs.log("class " + class + ": " + stats[class]);
    }
}
