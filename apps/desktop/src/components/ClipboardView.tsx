import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Clipboard, Code2, EyeOff, FileJson, Globe, KeyRound, Palette, Pin, Sparkles, Trash2 } from "lucide-react";

export type ClipboardKind = "text" | "code" | "json" | "url" | "color" | "jwt";

export type ClipboardItem = {
  id: string;
  content_type: ClipboardKind;
  content: string;
  preview: string;
  is_pinned: boolean;
  updated_at: number;
  formatted: string | null;
  language: string | null;
  color_swatch: string | null;
};

export type PauseStatus = {
  paused: boolean;
  mode: string;
  remaining_seconds: number | null;
};

const kindLabels: Record<ClipboardKind, string> = {
  text: "Texto",
  code: "Código",
  json: "JSON",
  url: "URL",
  color: "Color",
  jwt: "JWT",
};

const kindIcons: Record<ClipboardKind, typeof Clipboard> = {
  text: Clipboard,
  code: Code2,
  json: FileJson,
  url: Globe,
  color: Palette,
  jwt: KeyRound,
};

type Props = {
  query: string;
  selected: number;
  onSelect: (index: number) => void;
  onCountChange: (count: number) => void;
  onHide: () => Promise<void>;
  onError: (msg: string) => void;
  actionTrigger: { seq: number; kind: "copy" | "plain" | "pin" | "delete" } | null;
};

export function ClipboardView({
  query,
  selected,
  onSelect,
  onCountChange,
  onHide,
  onError,
  actionTrigger,
}: Props) {
  const [items, setItems] = useState<ClipboardItem[]>([]);
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [pause, setPause] = useState<PauseStatus>({ paused: false, mode: "active", remaining_seconds: null });
  const [pauseSelection, setPauseSelection] = useState("active");

  const reload = useCallback(async () => {
    try {
      const list = await invoke<ClipboardItem[]>("list_clipboard", { query, pinnedOnly });
      setItems(list);
      onCountChange(list.length);
    } catch (err) {
      onError(String(err));
    }
  }, [query, pinnedOnly, onCountChange, onError]);

  useEffect(() => {
    void reload();
  }, [reload]);

  useEffect(() => {
    invoke<PauseStatus>("clipboard_pause_status")
      .then(status => {
        setPause(status);
        if (!status.paused) setPauseSelection("active");
        else if (status.mode === "manual") setPauseSelection("manual");
      })
      .catch(() => {});
    const subClip = listen("clipboard-updated", () => {
      void reload();
    });
    const subPause = listen<PauseStatus>("pause-updated", event => {
      setPause(event.payload);
      if (!event.payload.paused) setPauseSelection("active");
      else if (event.payload.mode === "manual") setPauseSelection("manual");
    });
    return () => {
      for (const s of [subClip, subPause]) void s.then(u => u()).catch(() => {});
    };
  }, [reload]);

  const current = items[selected] ?? items[0];

  const handleCopy = useCallback(
    async (item: ClipboardItem | undefined, plain: boolean) => {
      if (!item) return;
      try {
        await invoke("copy_clipboard_item", { id: item.id, plain });
        await onHide();
      } catch (err) {
        onError(String(err));
      }
    },
    [onHide, onError],
  );

  const handleTogglePin = useCallback(
    async (item: ClipboardItem | undefined) => {
      if (!item) return;
      try {
        await invoke("toggle_pin_clipboard", { id: item.id });
        await reload();
      } catch (err) {
        onError(String(err));
      }
    },
    [reload, onError],
  );

  const handleDelete = useCallback(
    async (item: ClipboardItem | undefined) => {
      if (!item) return;
      try {
        await invoke("delete_clipboard_item", { id: item.id });
        onSelect(Math.max(0, selected - 1));
        await reload();
      } catch (err) {
        onError(String(err));
      }
    },
    [reload, onError, onSelect, selected],
  );

  useEffect(() => {
    if (!actionTrigger) return;
    if (actionTrigger.kind === "copy") void handleCopy(current, false);
    else if (actionTrigger.kind === "plain") void handleCopy(current, true);
    else if (actionTrigger.kind === "pin") void handleTogglePin(current);
    else if (actionTrigger.kind === "delete") void handleDelete(current);
  }, [actionTrigger]);

  async function changePauseMode(value: string) {
    setPauseSelection(value);
    try {
      const next =
        value === "5"
          ? await invoke<PauseStatus>("set_clipboard_pause", { minutes: 5, manual: false })
          : value === "15"
            ? await invoke<PauseStatus>("set_clipboard_pause", { minutes: 15, manual: false })
            : value === "manual"
              ? await invoke<PauseStatus>("set_clipboard_pause", { minutes: null, manual: true })
              : await invoke<PauseStatus>("set_clipboard_pause", { minutes: null, manual: false });
      setPause(next);
    } catch (err) {
      onError(String(err));
    }
  }

  async function clearUnpinned() {
    try {
      await invoke("clear_clipboard_history");
      onSelect(0);
      await reload();
    } catch (err) {
      onError(String(err));
    }
  }

  return (
    <div className="stage2-panel">
      <div className="toolbar">
        <button
          type="button"
          className="chip"
          aria-pressed={pinnedOnly}
          onClick={() => {
            setPinnedOnly(v => !v);
            onSelect(0);
          }}
        >
          <Pin size={13} /> Solo fijados
        </button>
        <label className="pause-control">
          <EyeOff size={13} />
          <select
            aria-label="Modo pausa del portapapeles"
            value={pauseSelection}
            onChange={e => void changePauseMode(e.target.value)}
          >
            <option value="active">Captura activa</option>
            <option value="5">Pausar 5 min</option>
            <option value="15">Pausar 15 min</option>
            <option value="manual">Pausa manual</option>
          </select>
        </label>
        {pause.paused && (
          <span className="pause-badge" role="status">
            Modo incógnito activo
          </span>
        )}
        <button type="button" className="chip danger-chip" onClick={() => void clearUnpinned()}>
          <Trash2 size={13} /> Limpiar historial
        </button>
      </div>

      {!items.length ? (
        <p className="empty" role="status">
          {query.trim() ? "Sin coincidencias en el portapapeles" : "El historial del portapapeles está vacío"}
        </p>
      ) : (
        <div className="split-view">
          <div role="listbox" aria-label="Historial de portapapeles" className="list split-list">
            {items.map((item, i) => {
              const Icon = kindIcons[item.content_type] ?? Clipboard;
              return (
                <div
                  role="option"
                  key={item.id}
                  aria-selected={selected === i}
                  className={"row " + item.content_type}
                  onMouseDown={e => e.preventDefault()}
                  onClick={() => onSelect(i)}
                  onDoubleClick={() => void handleCopy(item, false)}
                >
                  {item.content_type === "color" && item.color_swatch ? (
                    <span
                      className="tile swatch-tile"
                      aria-label={"Color " + item.color_swatch}
                      style={{ backgroundColor: item.color_swatch }}
                    />
                  ) : (
                    <span className="tile">
                      <Icon size={17} />
                    </span>
                  )}
                  <span className="description">
                    <strong>{item.preview}</strong>
                    <small>
                      {kindLabels[item.content_type]}
                      {item.language ? ` · ${item.language}` : ""}
                    </small>
                  </span>
                  {item.is_pinned && (
                    <span className="pin-badge" title="Fijado">
                      <Pin size={13} />
                    </span>
                  )}
                </div>
              );
            })}
          </div>

          {current && (
            <aside className="detail-pane" aria-label="Vista previa del portapapeles">
              <div className="detail-header">
                <span className="badge">
                  {kindLabels[current.content_type]}
                  {current.language ? ` · ${current.language}` : ""}
                </span>
                <div className="detail-actions">
                  <button
                    type="button"
                    className="chip"
                    aria-label={current.is_pinned ? "Desfijar elemento" : "Fijar elemento"}
                    onClick={() => void handleTogglePin(current)}
                  >
                    <Pin size={13} /> {current.is_pinned ? "Desfijar" : "Fijar"}
                  </button>
                  <button
                    type="button"
                    className="chip"
                    onClick={() => void handleCopy(current, true)}
                    title="Shift+Enter"
                  >
                    <Sparkles size={13} /> Pegado limpio
                  </button>
                  <button
                    type="button"
                    className="chip danger-chip"
                    aria-label="Eliminar elemento"
                    onClick={() => void handleDelete(current)}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              </div>
              {current.content_type === "color" && current.color_swatch && (
                <div className="color-banner">
                  <span
                    className="color-preview-box"
                    style={{ backgroundColor: current.color_swatch }}
                  />
                  <code>{current.color_swatch}</code>
                </div>
              )}
              <pre className="detail-code">{current.formatted ?? current.content}</pre>
            </aside>
          )}
        </div>
      )}
    </div>
  );
}
