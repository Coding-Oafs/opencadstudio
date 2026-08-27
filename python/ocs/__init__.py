"""OpenCADStudio Python API.

Scripts run in a separate CPython process and talk to the application over
one JSON-lines channel; every ``ocs.*`` function is one request dispatched
by the host through the same audited paths as the ribbon, the command
line, and Rhai macros.

Importing this package also redirects ``print`` to the application's
script console (stderr keeps raw passthrough for tracebacks).
"""

import json
import sys
import threading

# The protocol stream is the process stdout captured at import time,
# before the console wrapper replaces sys.stdout for user code.
_PROTOCOL_OUT = sys.stdout
_LOCK = threading.Lock()
_NEXT_ID = 0


def _write(payload):
    _PROTOCOL_OUT.write(json.dumps(payload, separators=(",", ":")) + "\n")
    _PROTOCOL_OUT.flush()


def _call(function, args):
    """Send one request and block for its matching reply."""
    global _NEXT_ID
    with _LOCK:
        _NEXT_ID += 1
        request_id = _NEXT_ID
        _write({"id": request_id, "function": function, "args": args})
        while True:
            line = sys.stdin.readline()
            if not line:
                raise RuntimeError(
                    "the OpenCADStudio host stopped responding"
                )
            reply = json.loads(line)
            if reply.get("id") == request_id:
                if reply.get("ok"):
                    return reply.get("value")
                raise RuntimeError(reply.get("error", "unknown error"))
            # Replies are strictly ordered; a mismatched id is a host bug.


class _Console:
    """Forwards print() to the application console via the protocol."""

    def write(self, text):
        if text.strip():
            _write({"print": text.rstrip("\n")})
        return len(text)

    def flush(self):
        pass


if not isinstance(sys.stdout, _Console):
    sys.stdout = _Console()


def log(message):
    """Print a line to the application script console."""
    _write({"print": str(message)})


def command(line):
    """Run one command-line command, e.g. command('POINTCLOUDSTATS')."""
    return _call("command", [str(line)])


def cloud_attach(path):
    """Attach one LAS/LAZ file; returns its dataset source id."""
    return _call("cloud_attach", [str(path)])


def cloud_attach_folder(path):
    """Attach every LAS/LAZ under a folder (queued); returns queued count."""
    return _call("cloud_attach_folder", [str(path)])


def cloud_sources():
    """Attached sources: [{id, path, points, displayed, edits}]."""
    return _call("cloud_sources", [])


def cloud_stats():
    """Per-class point counts over the current display working set."""
    return _call("cloud_stats", [])


def cloud_filter(filter_json):
    """Set the persistent attribute filter used by spatial selections."""
    return _call("cloud_filter", [str(filter_json)])


def cloud_select_slice(low, high):
    """Select points between two survey elevations."""
    return _call("cloud_select_slice", [float(low), float(high)])


def cloud_select_clear():
    """Clear the active selection in every source."""
    return _call("cloud_select_clear", [])


def cloud_classify_selection(classification):
    """Reclassify the active selection as one ASPRS class."""
    return _call("cloud_classify_selection", [int(classification)])


def cloud_classify(source_id, classification, indices):
    """Classify explicit source indices ('10,25-40') of one source."""
    return _call("cloud_classify", [str(source_id), int(classification), str(indices)])


def cloud_undo():
    """Undo the most recent point edit action."""
    return _call("cloud_undo", [])


def cloud_export_all(path):
    """Start a merged export of every source; returns immediately."""
    return _call("cloud_export_all", [str(path)])


def cloud_export_status():
    """Export/reprojection progress: {running, completed, total}."""
    return _call("cloud_export_status", [])


def cloud_detach():
    """Detach every attached source (session only; sources unchanged)."""
    return _call("cloud_detach", [])


def cloud_list_folder(path):
    """List the LAS/LAZ files directly under a folder (not recursive)."""
    return _call("cloud_list_folder", [str(path)])


def cloud_urban_classify(settings_json):
    """Start a native urban classification from a settings JSON preset."""
    return _call("cloud_urban_classify", [str(settings_json)])


def cloud_urban_status():
    """Urban job status: stage, tile, points, references, elapsed."""
    return _call("cloud_urban_status", [])


def cloud_urban_cancel():
    """Request cancellation of the running urban job."""
    return _call("cloud_urban_cancel", [])
