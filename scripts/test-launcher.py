#!/usr/bin/env python3
"""Prueba la aplicación Tauri empaquetada en un KWin virtual y un bus privado."""
import json
import os
from pathlib import Path
import socket
import statistics
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from contextlib import ExitStack


def wait_for(probe, description):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if result := probe():
            return result
        time.sleep(0.05)
    raise AssertionError(description)


def environment(root):
    env = dict(os.environ, XDG_RUNTIME_DIR=str(root), XDG_DATA_HOME=str(root / "data"),
               XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"),
               XDG_STATE_HOME=str(root / "state"), FLUXCAST_PROJECT_DIRS=str(root / "repos"),
               WAYLAND_DISPLAY="wayland-fluxcast-review", GDK_BACKEND="wayland",
               TAURI_WEBVIEW_AUTOMATION="true", QT_QPA_PLATFORM="wayland")
    env.pop("DISPLAY", None)
    return env


def review(binary, root):
    env = dict(os.environ)
    repository = root / "repos" / "Proyecto de revisión"
    (repository / ".git").mkdir(parents=True)
    applications = root / "data/applications"
    applications.mkdir(parents=True)
    (applications / "review.desktop").write_text(
        f"[Desktop Entry]\nType=Application\nName=FluxCastReviewApp\nExec=/usr/bin/touch {root}/opened\n")
    (root / "bin").mkdir()
    dolphin = root / "bin/dolphin"
    dolphin.write_text('#!/bin/sh\nprintf "%s" "$1" > "$XDG_RUNTIME_DIR/project-opened"\n')
    dolphin.chmod(0o755)
    env["PATH"] = str(root / "bin") + os.pathsep + env["PATH"]
    with socket.socket() as port_socket:
        port_socket.bind(("127.0.0.1", 0))
        port = port_socket.getsockname()[1]

    def api(method, path, data=None):
        body = None if data is None else json.dumps(data).encode()
        req = Request(f"http://127.0.0.1:{port}{path}", data=body, method=method,
                      headers={"Content-Type": "application/json"})
        try:
            with urlopen(req, timeout=20) as reply:
                return json.load(reply)["value"]
        except HTTPError as error:
            raise RuntimeError(error.read().decode()) from error

    with ExitStack() as stack:
        def start(args, **options):
            proc = subprocess.Popen(args, env=env, **options)
            def stop():
                proc.terminate()
                try:
                    proc.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
            stack.callback(stop)
        start(["kwin_wayland", "--virtual", "--socket", env["WAYLAND_DISPLAY"]])
        wait_for((root / env["WAYLAND_DISPLAY"]).exists, "KWin no creó el display")
        portal_log = root / "shortcut-request.log"
        log = stack.enter_context(portal_log.open("w"))
        start(["stdbuf", "-oL", "dbus-monitor", "--session",
               "interface='org.freedesktop.portal.GlobalShortcuts',member='BindShortcuts'"], stdout=log)
        start(["WebKitWebDriver", "-p", str(port)])
        def ready():
            try:
                return api("GET", "/status")["ready"]
            except OSError:
                return False
        wait_for(ready, "WebKitWebDriver no inició")
        capabilities = {"webkitgtk:browserOptions": {"binary": str(binary), "args": []}}
        session = "/session/" + api("POST", "/session", {"capabilities": {"alwaysMatch": capabilities}})["sessionId"]
        stack.callback(api, "DELETE", session)
        def js(script):
            return api("POST", session + "/execute/sync", {"script": script, "args": []})
        def keys(text):
            element = api("POST", session + "/element", {"using": "css selector", "value": "input"})
            api("POST", session + "/element/" + next(iter(element.values())) + "/value", {"text": text})
        def has_text(text):
            return js("return document.body.innerText").find(text) != -1
        wait_for(lambda: js("return !!document.querySelector('input')"), "No cargó React")
        wait_for(lambda: all(field in portal_log.read_text() for field in ('"toggle_launcher"', '"ALT+space"')),
                 "FluxCast no solicitó Alt+Space al portal KDE")
        keys("FluxCastReviewApp")
        wait_for(lambda: has_text("review.desktop"), "No se indexó la aplicación")
        keys("\ue007")
        wait_for((root / "opened").exists, "Enter no abrió la aplicación .desktop")
        subprocess.run([binary, "toggle"], check=True, env=env)
        wait_for(lambda: js("return document.querySelector('input').value === ''"), "El socket no reabrió el launcher")
        keys("Proyecto de revisión")
        wait_for(lambda: has_text(str(repository)), "No se indexó el proyecto")
        keys("\ue007")
        marker = root / "project-opened"
        wait_for(lambda: marker.exists() and marker.read_text() == str(repository), "Ruta de proyecto incorrecta")
        subprocess.run([binary, "toggle"], check=True, env=env)
        keys("= 17 * 3")
        wait_for(lambda: has_text("51"), "La calculadora no respondió")
        keys("\ue007")
        def clipboard_matches():
            read = subprocess.run(["wl-paste", "--no-newline"], env=env, capture_output=True, timeout=3)
            return read.returncode == 0 and read.stdout == b"51"
        wait_for(clipboard_matches, "Enter no copió el resultado en Wayland")
        timings = []
        for _ in range(21):
            before = time.perf_counter_ns()
            subprocess.run([binary, "toggle"], check=True, env=env)
            timings.append((time.perf_counter_ns() - before) / 1e6)
        print(f"CLI y cambio de visibilidad, p50 {statistics.median(timings):.2f} ms, p95 {sorted(timings)[19]:.2f} ms")
        # Usar el IPC real excluye el transporte de WebDriver de la medición.
        response = api("POST", session + "/execute/async", {"args": [], "script": """
            const done = arguments[arguments.length - 1];
            (async () => {
                const { invoke } = window.__TAURI_INTERNALS__;
                const times = [];
                for (let i = 0; i < 25; i++) {
                    const start = performance.now();
                    const results = await invoke("search", {query: "FluxCastReviewApp", scope: "app"});
                    await invoke("open_result", {id: results.hits[0].id, opener: "dolphin"});
                    times.push(performance.now() - start);
                }
                done(times);
            })().catch(e => done(String(e)));
        """})
        assert isinstance(response, list), response
        print(f"Búsqueda y despacho GIO por IPC, p50 {statistics.median(response):.2f} ms, p95 {sorted(response)[23]:.2f} ms")
        print("OK · KWin/Wayland, búsqueda, Enter, apertura de app y proyecto, socket y calculadora", flush=True)


if __name__ == "__main__":
    default_binary = Path("target/release/fluxcast")
    binary_arg = Path(sys.argv[1]) if len(sys.argv) > 1 else default_binary
    if not binary_arg.exists():
        raise SystemExit(
            f"No existe {binary_arg}. Compila primero con: pnpm --filter @fluxcast/desktop tauri build --no-bundle"
        )
    binary = binary_arg.resolve(strict=True)
    if len(sys.argv) == 4 and sys.argv[2] == "--session":
        review(binary, Path(sys.argv[3]))
    else:
        directory = Path(tempfile.mkdtemp(prefix="fluxcast-review-"))
        try:
            subprocess.run(["dbus-run-session", "--", sys.executable, __file__, str(binary), "--session", str(directory)],
                           env=environment(directory), check=True)
        finally:
            doc_mount = directory / "doc"
            for tool in ("fusermount3", "fusermount"):
                subprocess.run([tool, "-u", "-z", str(doc_mount)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            import shutil
            shutil.rmtree(directory, ignore_errors=True)
