import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AppWindow, ArrowDown, ArrowUp, Braces, Command, CornerDownLeft, FolderGit2, Power, Search, X, Zap } from "lucide-react";

type Scope = "all" | "app" | "project" | "tools";
type Hit = { id: number; name: string; detail: string; kind: "app" | "project" | "action" };
type Results = { hits: Hit[]; utility: { label: string; value: string } | null; problem: string | null };
type Row = Hit | { kind: "example" | "value"; name: string; detail: string };
const scopes: [Scope, string][] = [["all", "Todo"], ["app", "Aplicaciones"], ["project", "Proyectos"], ["tools", "Utilidades"]];
const openers = [["dolphin", "Dolphin"], ["code", "VS Code"], ["konsole", "Konsole"], ["neovim", "Neovim"]];
const examples = [
  ["Calculadora", "(1920 * 0.75) + 48"], ["Codificar Base64", "b64 FluxCast"], ["Decodificar Base64", "unb64 SG9sYQ=="],
  ["Generar UUID v4", "uuid4"], ["Generar UUID v7", "uuid7"], ["Convertir fecha Unix / ISO", "ts 1700000000"],
  ["Timestamp actual", "ts now"], ["Leer JWT", "jwt "], ["Hexadecimal", "hex 255"], ["Binario", "bin 10"], ["Decimal", "dec 0xFF"],
];
const kindLabel = { app: "Aplicación", project: "Proyecto Git", action: "Acción", example: "Utilidad", value: "Resultado" };
const icons = { app: AppWindow, project: FolderGit2, action: Power, example: Braces, value: Braces };

export function CommandBar() {
  const [query, setQuery] = useState("");
  const [scope, setScope] = useState<Scope>("all");
  const [catalogVersion, setCatalogVersion] = useState(0);
  const [response, setResponse] = useState<{ query: string; scope: Scope; version: number; data: Results } | null>(null);
  const [selected, select] = useState(0);
  const [opener, setOpener] = useState("dolphin");
  const [shortcut, setShortcut] = useState("");
  const [error, setError] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const busy = useRef(false);
  const data = response?.query === query && response.scope === scope && response.version === catalogVersion ? response.data : null;

  useEffect(() => {
    let active = true;
    setError("");
    invoke<Results>("search", { query, scope }).then(result => {
      if (active) setResponse({ query, scope, version: catalogVersion, data: result });
    }).catch(reason => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [query, scope, catalogVersion]);

  useEffect(() => {
    let active = true;
    const shown = listen("launcher-shown", () => {
      setScope("all"); setQuery(""); select(0); setError(""); input.current?.focus();
    });
    const status = listen<string>("shortcut-status", event => { if (active) setShortcut(event.payload); });
    status.then(() => invoke<string>("shortcut_status")).then(value => { if (active) setShortcut(value); }).catch(() => {});
    return () => {
      active = false;
      for (const sub of [shown, status]) void sub.then(unlisten => unlisten()).catch(() => {});
    };
  }, []);

  const rows: Row[] = [...(data?.hits ?? [])];
  if (data?.utility) rows.unshift({ kind: "value", name: data.utility.label, detail: data.utility.value });
  if (data && !data.utility && !data.problem && (scope === "tools" || (scope === "all" && query.trim()))) {
    const term = query.trim().toLowerCase();
    rows.push(...examples.filter(pair => pair.some(s => s.toLowerCase().includes(term)))
      .map(([name, detail]): Row => ({ kind: "example", name, detail })));
  }
  const current = rows[selected];
  useEffect(() => {
    list.current?.children[selected]?.scrollIntoView?.({ block: "nearest" });
  }, [selected, data]);

  function changeScope(next: Scope) {
    setScope(next); select(0); input.current?.focus();
  }
  async function refreshCatalog() {
    if (busy.current) return;
    busy.current = true;
    try {
      await invoke("refresh_catalog");
      select(0);
      setCatalogVersion(v => v + 1);
    } catch (reason) { setError(String(reason)); }
    finally { busy.current = false; }
  }
  async function activate(row?: Row, tool = opener) {
    if (!row || busy.current) return;
    if (row.kind === "example") {
      setQuery(row.detail); select(0); input.current?.focus(); return;
    }
    busy.current = true;
    try {
      if (row.kind === "value") await invoke("copy_value", { value: row.detail });
      else if ("id" in row) await invoke("open_result", { id: row.id, opener: tool });
      await invoke("hide_launcher");
    } catch (reason) { setError(String(reason)); }
    finally { busy.current = false; }
  }
  function onKey(event: KeyboardEvent) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault(); void invoke("hide_launcher").catch(e => setError(String(e))); return;
    }
    if (event.ctrlKey && event.key.toLowerCase() === "r") {
      event.preventDefault(); void refreshCatalog(); return;
    }
    const number = Number(event.key) - 1;
    if (event.ctrlKey && scopes[number]) { event.preventDefault(); changeScope(scopes[number][0]); return; }
    if (event.altKey && current?.kind === "project" && openers[number]) {
      event.preventDefault(); void activate(current, openers[number][0]); return;
    }
    if (event.target !== input.current) return;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (rows.length) select((selected + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length);
    } else if (event.key === "Enter") {
      event.preventDefault(); if (!event.repeat) void activate(current);
    }
  }

  return <main onKeyDown={onKey}>
    <header className="flex items-center gap-2">
      <span className="logo"><Zap size={17} fill="currentColor" /></span><strong>FluxCast</strong>
      <span className="shortcut" title="Atajo global de KDE o comando alternativo">{shortcut}</span>
      <button className="close" aria-label="Ocultar launcher" onClick={() => void invoke("hide_launcher").catch(e => setError(String(e)))}><X size={16}/></button>
    </header>
    <div className="search flex items-center gap-3">
      <Search size={23}/>
      <input ref={input} autoFocus role="combobox" aria-label="Buscar" aria-controls="results" aria-expanded="true"
        aria-autocomplete="list" aria-activedescendant={current ? 'result-' + selected : undefined}
        placeholder="Buscar apps, proyectos, acciones o comandos…" value={query}
        spellCheck={false} autoComplete="off" onChange={event => { setQuery(event.target.value); select(0); }}/>
    </div>
    <nav aria-label="Filtrar resultados" className="flex gap-1">
      {scopes.map(([key, label], i) => <button key={key} aria-pressed={scope === key} title={'Ctrl+' + (i+1)}
        onClick={() => changeScope(key)}>{label}</button>)}
    </nav>
    <section className="results">
      {error && <p role="alert" className="error">{error}</p>}
      <div id="results" role="listbox" aria-label="Resultados" className="list" ref={list}>
        {rows.map((row, i) => {
          const Icon = icons[row.kind];
          return <div role="option" id={'result-' + i} key={'id' in row ? row.id : row.name} aria-selected={selected === i}
            className={'row ' + row.kind} onMouseDown={e => e.preventDefault()} onClick={() => { select(i); void activate(row); }}>
            <span className="tile"><Icon size={18}/></span>
            <span className="description"><strong>{row.name}</strong><small title={row.detail}>{row.detail}</small></span>
            <span className="kind">{kindLabel[row.kind]}</span>
          </div>;
        })}
      </div>
      {!rows.length && !error && <p className="empty" role="status">{data?.problem ?? (data ? "Sin resultados" : "Buscando…")}</p>}
    </section>
    <footer className="flex items-center gap-3">
      <span className="flex items-center gap-1"><kbd><ArrowUp/></kbd><kbd><ArrowDown/></kbd> Navegar</span>
      {current?.kind === "project" && <select aria-label="Abrir proyecto con" value={opener} onChange={e => setOpener(e.target.value)}>
        {openers.map(([key, name], i) => <option key={key} value={key}>{name} · Alt+{i+1}</option>)}</select>}
      <span className="primary-action"><kbd><CornerDownLeft/></kbd> {current?.kind === "value" ? "Copiar" : current?.kind === "example" ? "Usar" : current?.kind === "action" ? "Ejecutar" : "Abrir"}</span>
      <span className="footer-brand" title="Ctrl+R actualiza el índice" aria-hidden="true"><Command size={15}/></span>
    </footer>
  </main>;
}
