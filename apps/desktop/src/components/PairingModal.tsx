import { useCallback, useEffect, useState, type FormEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { KeyRound, QrCode, RefreshCw, Server, ShieldCheck, Smartphone } from "lucide-react";

export type SyncStatus = {
  server_url: string;
  registered: boolean;
  user_id: string | null;
  device_id: string | null;
  last_seq: number;
  pending_outbox: number;
  keyring_backend: string;
  key_fingerprint: string;
};

export type PairingQrView = {
  qr_svg: string;
  payload_json: string;
  pairing_token: string;
  expires_at: number;
  server_url: string;
};

export type SyncCycleReport = {
  pushed: number;
  pulled: number;
  last_seq: number;
};

type Props = {
  onError: (msg: string) => void;
};

export function PairingModal({ onError }: Props) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [serverInput, setServerInput] = useState("http://127.0.0.1:8787");
  const [passphrase, setPassphrase] = useState("");
  const [qr, setQr] = useState<PairingQrView | null>(null);
  const [report, setReport] = useState<string>("");
  const [busy, setBusy] = useState(false);

  const reloadStatus = useCallback(async () => {
    try {
      const next = await invoke<SyncStatus>("sync_status");
      setStatus(next);
      setServerInput(next.server_url);
    } catch (err) {
      onError(String(err));
    }
  }, [onError]);

  useEffect(() => {
    void reloadStatus();
    const sub = listen("sync-updated", () => {
      void reloadStatus();
    });
    return () => {
      void sub.then(u => u()).catch(() => {});
    };
  }, [reloadStatus]);

  async function saveServer(e: FormEvent) {
    e.preventDefault();
    try {
      const next = await invoke<SyncStatus>("set_sync_server_url", { serverUrl: serverInput });
      setStatus(next);
      setReport("URL del servidor actualizada");
    } catch (err) {
      onError(String(err));
    }
  }

  async function derivePassphrase(e: FormEvent) {
    e.preventDefault();
    if (!passphrase.trim()) return;
    try {
      const next = await invoke<SyncStatus>("rotate_sync_passphrase", { passphrase });
      setStatus(next);
      setPassphrase("");
      setReport("Master Key derivada con Argon2id y guardada en KWallet");
    } catch (err) {
      onError(String(err));
    }
  }

  async function runSyncNow() {
    if (busy) return;
    setBusy(true);
    try {
      const cycle = await invoke<SyncCycleReport>("sync_now");
      setReport(`Sync E2EE completado · subidos: ${cycle.pushed}, recibidos: ${cycle.pulled} (seq ${cycle.last_seq})`);
      await reloadStatus();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(false);
    }
  }

  async function generateQr() {
    if (busy) return;
    setBusy(true);
    try {
      const view = await invoke<PairingQrView>("generate_pairing_qr");
      setQr(view);
      setReport("Código QR efímero generado (válido por 120 s)");
      await reloadStatus();
    } catch (err) {
      onError(String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="stage2-panel">
      <div className="toolbar">
        <span className="chip accent-chip">
          <ShieldCheck size={13} /> E2EE Zero-Knowledge · Argon2id + XChaCha20-Poly1305
        </span>
        {status && (
          <span className="chip" title="Almacenamiento de Master Key">
            <KeyRound size={12} /> {status.keyring_backend} · #{status.key_fingerprint}
          </span>
        )}
        <button type="button" className="chip accent-chip" onClick={() => void runSyncNow()} disabled={busy}>
          <RefreshCw size={12} /> Sincronizar ahora ({status?.pending_outbox ?? 0} en cola)
        </button>
        <button type="button" className="chip" onClick={() => void generateQr()} disabled={busy}>
          <QrCode size={12} /> Vincular Android por QR
        </button>
      </div>

      <div className="split-view">
        <div className="snippet-form">
          <form onSubmit={e => void saveServer(e)} className="sync-card">
            <strong>
              <Server size={14} /> Servidor Sync Relay (Self-Hosted)
            </strong>
            <div className="form-row">
              <input
                aria-label="URL del servidor Sync Relay"
                value={serverInput}
                onChange={e => setServerInput(e.target.value)}
                placeholder="http://127.0.0.1:8787"
              />
              <button type="submit" className="chip">
                Guardar servidor
              </button>
            </div>
            <small>
              Cursor delta: seq {status?.last_seq ?? 0} · Outbox pendiente: {status?.pending_outbox ?? 0}
            </small>
          </form>

          <form onSubmit={e => void derivePassphrase(e)} className="sync-card">
            <strong>
              <KeyRound size={14} /> Derivar Master Key con Argon2id (KWallet / ksecretd)
            </strong>
            <div className="form-row">
              <input
                type="password"
                aria-label="Frase de paso Argon2id"
                value={passphrase}
                onChange={e => setPassphrase(e.target.value)}
                placeholder="Nueva frase de paso (mín. 6 caracteres)"
              />
              <button type="submit" className="chip">
                Derivar llave
              </button>
            </div>
          </form>

          {report && (
            <p className="badge" role="status">
              {report}
            </p>
          )}
        </div>

        <aside className="detail-pane" aria-label="Vinculación QR con Android">
          <div className="detail-header">
            <span className="badge">
              <Smartphone size={13} /> Emparejamiento Android (120 s)
            </span>
          </div>
          {qr ? (
            <div className="qr-box">
              <div
                className="qr-svg"
                aria-label="Código QR de vinculación"
                dangerouslySetInnerHTML={{ __html: qr.qr_svg }}
              />
              <small>Token: {qr.pairing_token}</small>
            </div>
          ) : (
            <p className="empty">
              Pulsa «Vincular Android por QR» para generar un token de un solo uso (120 s) con tu Master Key E2EE.
            </p>
          )}
        </aside>
      </div>
    </div>
  );
}
