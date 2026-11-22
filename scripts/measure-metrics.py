#!/usr/bin/env python3
"""Mide métricas reales de la Etapa 5: RAM RSS, latencia de sync E2EE y verifica el instalador."""
import json
import os
from pathlib import Path
import socket
import statistics
import subprocess
import tempfile
import time
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parent.parent


def read_rss_mb(pid: int) -> float:
    status_file = Path(f"/proc/{pid}/status")
    for line in status_file.read_text().splitlines():
        if line.startswith("VmRSS:"):
            kb = int(line.split()[1])
            return kb / 1024.0
    return 0.0


def verify_installer() -> None:
    with tempfile.TemporaryDirectory(prefix="fluxcast-install-test-") as tmp:
        tmp_path = Path(tmp)
        env = dict(
            os.environ,
            FLUXCAST_INSTALL_PREFIX=str(tmp_path / "local"),
            FLUXCAST_SYSTEMD_USER_DIR=str(tmp_path / "systemd"),
        )
        subprocess.run(
            [str(ROOT / "scripts/install-fedora-kde.sh"), "--no-systemd"],
            cwd=ROOT,
            env=env,
            check=True,
            stdout=subprocess.DEVNULL,
        )
        assert (tmp_path / "local/bin/fluxcast").is_file()
        assert (tmp_path / "local/bin/fluxcast-desktop").is_file()
        assert (tmp_path / "systemd/fluxcast.service").is_file()
        assert (tmp_path / "local/share/applications/dev.fluxcast.desktop.desktop").is_file()


def measure_sync_and_ram() -> None:
    server_bin = ROOT / "target/debug/fluxcast-sync-server"
    if not server_bin.exists():
        subprocess.run(["cargo", "build", "-p", "fluxcast-sync-server"], cwd=ROOT, check=True)

    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]

    env = dict(os.environ, FLUXCAST_BIND=f"127.0.0.1:{port}")
    proc = subprocess.Popen(
        [str(server_bin)],
        cwd=ROOT,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        base = f"http://127.0.0.1:{port}"
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            try:
                with urlopen(f"{base}/api/v1/health", timeout=1) as r:
                    if r.status == 200:
                        break
            except OSError:
                time.sleep(0.05)

        def api(method: str, path: str, body=None, token=None):
            headers = {"Content-Type": "application/json"}
            if token:
                headers["Authorization"] = f"Bearer {token}"
            data = None if body is None else json.dumps(body).encode()
            req = Request(f"{base}{path}", data=data, method=method, headers=headers)
            with urlopen(req, timeout=5) as resp:
                return json.load(resp)

        session = api(
            "POST",
            "/api/v1/auth/register",
            {"user_id": None, "device_name": "Fedora Benchmark", "platform": "fedora-kde"},
        )
        token = session["access_token"]
        device_id = session["device_id"]

        latencies_ms = []
        for i in range(35):
            entity_id = f"019bf62b-e614-7000-8000-{i:012x}"
            hlc = f"{1700000000000 + i:015d}-0001-fedora"
            t0 = time.perf_counter_ns()
            api(
                "POST",
                "/api/v1/sync/push",
                {
                    "blobs": [
                        {
                            "entity_id": entity_id,
                            "entity_kind": 1,
                            "device_id": device_id,
                            "hlc_timestamp": hlc,
                            "dedup_hmac": [i % 251] * 32,
                            "nonce": [1] * 24,
                            "ciphertext": [42] * 96,
                            "is_deleted": False,
                        }
                    ]
                },
                token=token,
            )
            pulled = api("GET", f"/api/v1/sync/pull?since_seq={i}", token=token)
            assert len(pulled["blobs"]) == 1
            latencies_ms.append((time.perf_counter_ns() - t0) / 1e6)

        rss_mb = read_rss_mb(proc.pid)
        latencies_ms.sort()
        p50 = statistics.median(latencies_ms)
        p95 = latencies_ms[int(len(latencies_ms) * 0.95)]
        print(
            f"Métricas Etapa 5 · Sync Relay Server RSS: {rss_mb:.2f} MB | "
            f"Delta Push+Pull E2EE p50: {p50:.2f} ms, p95: {p95:.2f} ms"
        )
        assert rss_mb < 50.0, f"RSS demasiado alto: {rss_mb} MB"
        assert p95 < 50.0, f"Latencia p95 demasiado alta: {p95} ms"
    finally:
        proc.terminate()
        proc.wait(timeout=5)


if __name__ == "__main__":
    verify_installer()
    measure_sync_and_ram()
    print("OK · Instalador systemd --user y métricas reales de Etapa 5 verificadas")
