import { useState } from "react";
import type { Api, MappingFile } from "../api";
import { errorMessage } from "../api";

interface Props {
  api: Pick<Api, "pickMappingFile">;
  /** The picked file's text; the caller imports it like pasted CSV. */
  onLoaded: (file: MappingFile) => void;
  onError: (message: string) => void;
  /** The file currently loaded, or null after the text was edited by hand. */
  file: MappingFile | null;
  disabled?: boolean;
}

/** Opens the native file dialog for a mapping CSV; Rust reads the file. */
export function MappingFileButton({ api, onLoaded, onError, file, disabled }: Props) {
  const [busy, setBusy] = useState(false);

  async function choose() {
    setBusy(true);
    try {
      const picked = await api.pickMappingFile();
      if (picked) {
        onLoaded(picked);
      }
    } catch (err) {
      onError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-w-0 items-center gap-2 text-xs text-slate-400">
      <button
        type="button"
        className="shrink-0 rounded border border-slate-700 px-3 py-1.5 text-sm text-slate-200 hover:bg-slate-800 disabled:opacity-50"
        disabled={disabled || busy}
        onClick={() => void choose()}
      >
        Choose file…
      </button>
      {file ? (
        <span className="truncate" title={file.path}>
          Loaded <span className="font-mono text-slate-300">{file.name}</span>
        </span>
      ) : (
        <span>or paste the CSV above.</span>
      )}
    </div>
  );
}
