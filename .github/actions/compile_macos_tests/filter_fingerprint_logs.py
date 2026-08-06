import re
import sys


ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*[mK]")
CARGO_STATUS = re.compile(
    r"^\s*(?:"
    r"Compiling|Checking|Finished|Fresh|Dirty|Building|Blocking|"
    r"Downloading|Downloaded|Updating|Locking|Adding|Removing|"
    r"Packaging|Verifying|Archiving|Installing|Installed|Running|"
    r"Doc-tests|Executable"
    r")\b"
)
FINGERPRINT_LOGGER = "cargo::core::compiler::fingerprint:"


def write_live(line: str) -> None:
    sys.stdout.write(line)
    sys.stdout.flush()


def main() -> None:
    suppress_continuation = False
    in_cause = False

    for line in sys.stdin:
        plain = ANSI_ESCAPE.sub("", line).rstrip("\n")

        if FINGERPRINT_LOGGER in plain:
            suppress_continuation = True
            in_cause = False
            continue

        if not suppress_continuation:
            write_live(line)
            continue

        if CARGO_STATUS.match(plain) or plain.startswith(("error", "warning")):
            suppress_continuation = False
            in_cause = False
            write_live(line)
        elif not plain:
            continue
        elif plain == "Caused by:":
            in_cause = True
        elif in_cause and line[:1].isspace():
            continue
        else:
            suppress_continuation = False
            in_cause = False
            write_live(line)


if __name__ == "__main__":
    main()
