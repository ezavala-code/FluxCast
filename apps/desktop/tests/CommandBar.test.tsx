// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CommandBar } from "../src/components/CommandBar";

const { ipc } = vi.hoisted(() => ({ ipc: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: ipc }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
const app = { id: 8, kind: "app", name: "Kate", detail: "kate.desktop" };
const repo = { id: 12, kind: "project", name: "FluxCast", detail: "/dev/FluxCast" };
const results = { hits: [app, repo], utility: null, problem: null };

describe("CommandBar", () => {
  beforeEach(() => {
    ipc.mockReset().mockImplementation(async (command) => command === "search" ? results : "");
  });
  afterEach(cleanup);

  it("permite elegir un proyecto y abrirlo en Konsole desde el teclado", async () => {
    const user = userEvent.setup();
    render(<CommandBar/>);
    await screen.findByText("Kate");
    await user.keyboard("{ArrowDown}{Alt>}3{/Alt}");
    expect(ipc).toHaveBeenCalledWith("open_result", { id: 12, opener: "konsole" });
    expect(ipc).toHaveBeenCalledWith("hide_launcher");
  });

  it("copia el mismo valor mostrado y después oculta la ventana", async () => {
    const user = userEvent.setup();
    const value = "019bf62b-e614-7000-8000-000000000001";
    ipc.mockImplementation(async cmd => cmd === "search" ?
      { hits: [], utility: { label: "UUID", value }, problem: null } : "");
    render(<CommandBar/>);
    await screen.findByText(value);
    await user.keyboard("{Enter}");
    expect(ipc).toHaveBeenCalledWith("copy_value", { value });
    expect(ipc).toHaveBeenCalledWith("hide_launcher");
  });

  it("conserva la búsqueda más reciente aunque el índice responda tarde", async () => {
    const user = userEvent.setup();
    let finish!: (result: typeof results) => void;
    ipc.mockImplementation(async (cmd, args) => {
      if (cmd !== "search") return "";
      if (args.query === "k") return new Promise(resolve => { finish = resolve; });
      return args.query ? { ...results, hits: [] } : results;
    });
    render(<CommandBar/>);
    await screen.findByText("Kate");
    await user.type(screen.getByRole("combobox"), "kz");
    await screen.findByText("Sin resultados");
    await act(async () => finish(results));
    await user.keyboard("{Enter}");
    expect(screen.queryByText("Kate")).toBeNull();
    expect(ipc).not.toHaveBeenCalledWith("open_result", expect.anything());
  });

  it("mantiene abierta la ventana al fallar una acción y permite salir con Escape", async () => {
    const user = userEvent.setup();
    ipc.mockImplementation(async cmd => {
      if (cmd === "open_result") throw Error("Aplicación eliminada");
      return cmd === "search" ? results : "";
    });
    render(<CommandBar/>);
    await screen.findByText("Kate");
    await user.keyboard("{Enter}");
    expect((await screen.findByRole("alert")).textContent).toContain("Aplicación eliminada");
    expect(ipc).not.toHaveBeenCalledWith("hide_launcher");
    await user.keyboard("{Escape}");
    expect(ipc).toHaveBeenCalledWith("hide_launcher");
  });

  it("accede a las utilidades con Ctrl+4 y usa una plantilla", async () => {
    const user = userEvent.setup();
    ipc.mockImplementation(async cmd => cmd === "search" ? { ...results, hits: [] } : "");
    render(<CommandBar/>);
    await user.keyboard("{Control>}4{/Control}");
    await screen.findByText("Calculadora");
    await user.keyboard("{Enter}");
    await waitFor(() => expect(ipc).toHaveBeenCalledWith("search", { query: "(1920 * 0.75) + 48", scope: "tools" }));
  });

  it("actualiza el catálogo con Ctrl+R y ejecuta acciones rápidas de Plasma", async () => {
    const user = userEvent.setup();
    const action = { id: 99, kind: "action", name: "Bloquear sesión", detail: "Bloquea la sesión actual de Plasma (loginctl)" };
    ipc.mockImplementation(async cmd => cmd === "search" ? { hits: [action], utility: null, problem: null } : "");
    render(<CommandBar/>);
    await screen.findByText("Bloquear sesión");
    expect(screen.getByText("Ejecutar")).toBeTruthy();
    await user.keyboard("{Control>}r{/Control}");
    await waitFor(() => expect(ipc).toHaveBeenCalledWith("refresh_catalog"));
    await user.keyboard("{Enter}");
    expect(ipc).toHaveBeenCalledWith("open_result", { id: 99, opener: "dolphin" });
    expect(ipc).toHaveBeenCalledWith("hide_launcher");
  });

  it("gestiona el historial de portapapeles con Ctrl+5, fijado y pegado limpio con Shift+Enter", async () => {
    const user = userEvent.setup();
    const clipItem = {
      id: "clip-1",
      content_type: "json",
      content: "{\"kde\":6}",
      preview: "{\"kde\":6}",
      is_pinned: false,
      updated_at: 100,
      formatted: "{\n  \"kde\": 6\n}",
      language: "json",
      color_swatch: null,
    };
    ipc.mockImplementation(async cmd => {
      if (cmd === "list_clipboard") return [clipItem];
      if (cmd === "clipboard_pause_status") return { paused: false, mode: "active", remaining_seconds: null };
      if (cmd === "search") return results;
      return "";
    });
    render(<CommandBar/>);
    await user.keyboard("{Control>}5{/Control}");
    await screen.findByText("{\"kde\":6}");
    await user.keyboard("{Control>}p{/Control}");
    await waitFor(() => expect(ipc).toHaveBeenCalledWith("toggle_pin_clipboard", { id: "clip-1" }));
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await waitFor(() => expect(ipc).toHaveBeenCalledWith("copy_clipboard_item", { id: "clip-1", plain: true }));
    expect(ipc).toHaveBeenCalledWith("hide_launcher");
  });

  it("expande y crea snippets en la bóveda con Ctrl+6", async () => {
    const user = userEvent.setup();
    const snippet = {
      id: "snip-1",
      title: "Limpiar Docker",
      keyword: "!dprune",
      body: "docker system prune -af",
      language: "shell",
      tags: ["docker", "fedora"],
      use_count: 4,
      updated_at: 200,
    };
    ipc.mockImplementation(async cmd => {
      if (cmd === "list_snippets") return [snippet];
      if (cmd === "search") return results;
      return "";
    });
    render(<CommandBar/>);
    await user.keyboard("{Control>}6{/Control}");
    await screen.findByText(/Limpiar Docker/);
    await user.keyboard("{Enter}");
    await waitFor(() => expect(ipc).toHaveBeenCalledWith("use_snippet", { id: "snip-1" }));
    expect(ipc).toHaveBeenCalledWith("hide_launcher");
  });
});
