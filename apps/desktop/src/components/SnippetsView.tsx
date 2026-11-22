import { useCallback, useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Braces, Edit3, Flame, Plus, Tag, Trash2 } from "lucide-react";

export type Snippet = {
  id: string;
  title: string;
  keyword: string | null;
  body: string;
  language: string;
  tags: string[];
  use_count: number;
  updated_at: number;
};

type Props = {
  query: string;
  selected: number;
  onSelect: (index: number) => void;
  onCountChange: (count: number) => void;
  onHide: () => Promise<void>;
  onError: (msg: string) => void;
  actionTrigger: { seq: number; kind: "use" | "new" | "edit" | "delete" } | null;
};

const defaultTags = ["git", "docker", "fedora", "sql", "kotlin", "rust"];

export function SnippetsView({
  query,
  selected,
  onSelect,
  onCountChange,
  onHide,
  onError,
  actionTrigger,
}: Props) {
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [activeTag, setActiveTag] = useState<string | null>(null);
  const [editing, setEditing] = useState<{
    id: string | null;
    title: string;
    keyword: string;
    language: string;
    tagsText: string;
    body: string;
  } | null>(null);

  const reload = useCallback(async () => {
    try {
      const list = await invoke<Snippet[]>("list_snippets", { query, tag: activeTag });
      setSnippets(list);
      onCountChange(list.length);
    } catch (err) {
      onError(String(err));
    }
  }, [query, activeTag, onCountChange, onError]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const current = snippets[selected] ?? snippets[0];

  const handleUse = useCallback(
    async (snippet: Snippet | undefined) => {
      if (!snippet) return;
      try {
        await invoke("use_snippet", { id: snippet.id });
        await onHide();
      } catch (err) {
        onError(String(err));
      }
    },
    [onHide, onError],
  );

  const startNew = useCallback(() => {
    setEditing({
      id: null,
      title: "",
      keyword: "",
      language: "shell",
      tagsText: activeTag ?? "fedora",
      body: "",
    });
  }, [activeTag]);

  const startEdit = useCallback((snippet: Snippet | undefined) => {
    if (!snippet) return;
    setEditing({
      id: snippet.id,
      title: snippet.title,
      keyword: snippet.keyword ?? "",
      language: snippet.language,
      tagsText: snippet.tags.join(", "),
      body: snippet.body,
    });
  }, []);

  const handleDelete = useCallback(
    async (snippet: Snippet | undefined) => {
      if (!snippet) return;
      try {
        await invoke("delete_snippet", { id: snippet.id });
        onSelect(Math.max(0, selected - 1));
        await reload();
      } catch (err) {
        onError(String(err));
      }
    },
    [reload, onError, onSelect, selected],
  );

  useEffect(() => {
    if (!actionTrigger || editing) return;
    if (actionTrigger.kind === "use") void handleUse(current);
    else if (actionTrigger.kind === "new") startNew();
    else if (actionTrigger.kind === "edit") startEdit(current);
    else if (actionTrigger.kind === "delete") void handleDelete(current);
  }, [actionTrigger]);

  async function submitForm(event: FormEvent) {
    event.preventDefault();
    if (!editing) return;
    try {
      await invoke("save_snippet", {
        input: {
          id: editing.id,
          title: editing.title,
          keyword: editing.keyword.trim() || null,
          language: editing.language,
          tags: editing.tagsText
            .split(",")
            .map(s => s.trim())
            .filter(Boolean),
          body: editing.body,
        },
      });
      setEditing(null);
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
          aria-pressed={activeTag === null}
          onClick={() => {
            setActiveTag(null);
            onSelect(0);
          }}
        >
          Todos
        </button>
        {defaultTags.map(tag => (
          <button
            key={tag}
            type="button"
            className="chip"
            aria-pressed={activeTag === tag}
            onClick={() => {
              setActiveTag(prev => (prev === tag ? null : tag));
              onSelect(0);
            }}
          >
            <Tag size={11} /> {tag}
          </button>
        ))}
        <button type="button" className="chip accent-chip" onClick={startNew}>
          <Plus size={13} /> Nuevo snippet
        </button>
      </div>

      {editing ? (
        <form className="snippet-form" onSubmit={e => void submitForm(e)}>
          <div className="form-row">
            <input
              aria-label="Título del snippet"
              placeholder="Título (ej. Limpiar contenedores)"
              value={editing.title}
              onChange={e => setEditing({ ...editing, title: e.target.value })}
              required
            />
            <input
              aria-label="Alias o keyword"
              placeholder="Alias (ej. !dprune)"
              value={editing.keyword}
              onChange={e => setEditing({ ...editing, keyword: e.target.value })}
            />
          </div>
          <div className="form-row">
            <input
              aria-label="Etiquetas del snippet"
              placeholder="Etiquetas separadas por coma (git, docker, fedora)"
              value={editing.tagsText}
              onChange={e => setEditing({ ...editing, tagsText: e.target.value })}
            />
            <input
              aria-label="Lenguaje del snippet"
              placeholder="Lenguaje (shell, rust, sql, kotlin)"
              value={editing.language}
              onChange={e => setEditing({ ...editing, language: e.target.value })}
            />
          </div>
          <textarea
            aria-label="Cuerpo del snippet"
            placeholder="Soporta {{clipboard}}, {{date:YYYY-MM-DD}} y {{uuid}}"
            rows={5}
            value={editing.body}
            onChange={e => setEditing({ ...editing, body: e.target.value })}
            required
          />
          <div className="form-actions">
            <button type="button" className="chip" onClick={() => setEditing(null)}>
              Cancelar
            </button>
            <button type="submit" className="chip accent-chip">
              Guardar snippet
            </button>
          </div>
        </form>
      ) : !snippets.length ? (
        <p className="empty" role="status">
          No hay snippets para este filtro
        </p>
      ) : (
        <div className="split-view">
          <div role="listbox" aria-label="Bóveda de snippets" className="list split-list">
            {snippets.map((snippet, i) => (
              <div
                role="option"
                key={snippet.id}
                aria-selected={selected === i}
                className="row snippet"
                onMouseDown={e => e.preventDefault()}
                onClick={() => onSelect(i)}
                onDoubleClick={() => void handleUse(snippet)}
              >
                <span className="tile">
                  <Braces size={17} />
                </span>
                <span className="description">
                  <strong>
                    {snippet.title}
                    {snippet.keyword ? ` · ${snippet.keyword}` : ""}
                  </strong>
                  <small>{snippet.tags.join(" · ") || snippet.language}</small>
                </span>
                <span className="kind" title="Frecuencia de uso">
                  <Flame size={11} /> {snippet.use_count}
                </span>
              </div>
            ))}
          </div>

          {current && (
            <aside className="detail-pane" aria-label="Detalle del snippet">
              <div className="detail-header">
                <span className="badge">
                  {current.keyword ?? current.language} · {current.use_count} usos
                </span>
                <div className="detail-actions">
                  <button
                    type="button"
                    className="chip accent-chip"
                    onClick={() => void handleUse(current)}
                  >
                    Expandir y copiar
                  </button>
                  <button
                    type="button"
                    className="chip"
                    aria-label="Editar snippet"
                    onClick={() => startEdit(current)}
                  >
                    <Edit3 size={13} /> Editar
                  </button>
                  <button
                    type="button"
                    className="chip danger-chip"
                    aria-label="Eliminar snippet"
                    onClick={() => void handleDelete(current)}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              </div>
              <pre className="detail-code">{current.body}</pre>
            </aside>
          )}
        </div>
      )}
    </div>
  );
}
