"""Linux CLI credential delivery tests. Never print generated credentials."""
import base64
import errno
import fcntl
import hashlib
import os
from pathlib import Path
import pty
import re
import sqlite3
import subprocess
import sys
import termios


def run(binary, arguments, terminal=False):
    master, slave = pty.openpty() if terminal else (None, None)

    def attach_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    try:
        child = subprocess.Popen(
            [binary, *arguments], stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            pass_fds=(slave,) if terminal else (),
            preexec_fn=attach_terminal if terminal else None,
            start_new_session=not terminal,
        )
        if slave is not None:
            os.close(slave)
            slave = None
        try:
            stdout, stderr = child.communicate(timeout=15)
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate()
            raise AssertionError("Credential command failed to terminate") from None
        delivered = b""
        if terminal:
            while True:
                try:
                    chunk = os.read(master, 4096)
                    if not chunk:
                        break
                    delivered += chunk
                    assert len(delivered) < 65536, "Unexpected terminal output size"
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
        return child.returncode, stdout + stderr, delivered
    finally:
        for descriptor in (master, slave):
            if descriptor is not None:
                os.close(descriptor)


def credential(result):
    code, log, delivered = result
    assert code == 0, "Interactive credential command failed"
    match = re.search(r"：([A-Za-z0-9_.-]{20,})", delivered.decode("utf-8"))
    assert match, "Credential missing from controlling terminal"
    value = match.group(1)
    assert value.encode() not in log, "Credential leaked to stdout or stderr"
    return value


def admin_hash(path):
    with sqlite3.connect(path) as connection:
        return connection.execute("SELECT value FROM settings WHERE key='admin_hash'").fetchone()


def main():
    binary, directory = sys.argv[1:]
    directory = Path(directory)
    path = str(directory / "credential-tests.db")
    assert run(binary, ["admin", "init", "--db", path])[0] != 0
    assert admin_hash(path) is None, "Noninteractive init generated an unusable credential"
    result = run(binary, ["--listen", "127.0.0.1:0", "--db", path])
    assert result[0] != 0 and admin_hash(path) is None, "Daemon must not initialize credentials"
    initial = credential(run(binary, ["admin", "init", "--db", path], True))
    original = admin_hash(path)
    assert original[0].startswith("$argon2id$"), "Initial password must use Argon2id"
    assert run(binary, ["admin", "init", "--db", path])[0] == 0
    assert admin_hash(path) == original, "Idempotent init changed the administrator"
    assert run(binary, ["admin", "reset", "--db", path])[0] != 0
    assert admin_hash(path) == original, "Noninteractive reset changed the administrator"
    replacement = credential(run(binary, ["admin", "reset", "--db", path], True))
    assert replacement != initial and admin_hash(path) != original
    arguments = ["node", "create", "--id", "cli-test", "--name", "CLI test", "--db", path]
    assert run(binary, arguments)[0] != 0
    with sqlite3.connect(path) as connection:
        assert connection.execute("SELECT COUNT(*) FROM nodes").fetchone()[0] == 0
    node_key = credential(run(binary, arguments, True))
    with sqlite3.connect(path) as connection:
        stored = connection.execute("SELECT token_hash FROM nodes WHERE id='cli-test'").fetchone()[0]
        assert hashlib.sha256(node_key.encode()).digest() == stored

    # Seed the integration fixture with the v0.3.x credential format to exercise
    # an actual legacy login followed by migration to an Argon2id web password.
    fixture = str(directory / "monitor.db")
    value = credential(run(binary, ["admin", "init", "--db", fixture], True))
    encoded = base64.urlsafe_b64encode(hashlib.sha256(value.encode()).digest()).rstrip(b"=").decode()
    with sqlite3.connect(fixture) as connection:
        connection.execute("UPDATE settings SET value=? WHERE key='admin_hash'", (encoded,))
    descriptor = os.open(directory / "admin-test.token", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as fixture_file:
        fixture_file.write(value)
    print("PASS: terminal-only credentials, no log disclosure, idempotent init, safe noninteractive failure")


if __name__ == "__main__":
    main()
