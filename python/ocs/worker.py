"""Worker entry point: runs one script inside the OpenCADStudio host.

Usage (from the application host):

    python -u -m ocs.worker <script.py>

stdout is reserved for the JSON-lines protocol; stderr carries raw
tracebacks and diagnostics that the host prefixes into the console.
"""

import sys


def main(argv):
    if len(argv) != 2:
        print("usage: python -m ocs.worker <script.py>", file=sys.stderr)
        return 2
    script_path = argv[1]
    import ocs  # noqa: F401 - installs the console + protocol hooks

    with open(script_path, "r", encoding="utf-8") as stream:
        source = stream.read()
    code = compile(source, script_path, "exec")
    exec(code, {"__name__": "__main__"})
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
