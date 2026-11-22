#!/usr/bin/env python3
"""Prueba la aplicación Tauri empaquetada en un KWin virtual y un bus privado."""
import json
import os
from pathlib import Path
import socket
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
        def start(args):
            proc = subprocess.Popen(args, env=env)
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
        print("OK · KWin/Wayland, búsqueda, Enter, apertura de app y proyecto, socket y calculadora", flush=True)


if __name__ == "__main__":
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/fluxcast").resolve(strict=True)
    if len(sys.argv) == 4 and sys.argv[2] == "--session":
        review(binary, Path(sys.argv[3]))
    else:
        with tempfile.TemporaryDirectory(prefix="fluxcast-review-") as directory:
            subprocess.run(["dbus-run-session", "--", sys.executable, __file__, str(binary), "--session", directory],
                           env=environment(Path(directory)), check=True)
