import type { SlackWorkspace } from "../api";
import { findWorkspace, workspaceLabel } from "./workspaces";

interface Props {
  workspaces: SlackWorkspace[];
  /** A workspace domain, or "" when none is connected. */
  value: string;
  onChange: (domain: string) => void;
  label: string;
  className?: string;
  /** Opens wherever Slack gets connected; shown when no workspace is. */
  onConnect?: () => void;
  connectLabel?: string;
  /** Lets a row keep a value that is no longer connected. */
  allowUnlisted?: boolean;
}

/** Connected Slack workspaces as a select; the value is the domain mappings store. */
export function WorkspaceSelect({ workspaces, value, onChange, label, className, onConnect, connectLabel = "Connect Slack", allowUnlisted }: Props) {
  const unlisted = allowUnlisted && value.trim() !== "" && !findWorkspace(workspaces, value) ? value : null;
  const selected = findWorkspace(workspaces, value)?.domain ?? unlisted ?? "";
  if (workspaces.length === 0 && !unlisted) {
    return (
      <span className="inline-flex flex-wrap items-center gap-2">
        <select aria-label={label} className={className} disabled value="">
          <option value="">No Slack workspace connected</option>
        </select>
        {onConnect && (
          <>
            <span className="text-xs text-slate-400">Connect Slack to pick a workspace.</span>
            <button type="button" className="text-xs text-violet-300 underline" onClick={onConnect}>
              {connectLabel}
            </button>
          </>
        )}
      </span>
    );
  }
  return (
    <select aria-label={label} className={className} value={selected} onChange={(event) => onChange(event.target.value)}>
      {unlisted && <option value={unlisted}>{`${unlisted} · not connected`}</option>}
      {workspaces.map((item) => (
        <option key={item.id} value={item.domain}>
          {workspaceLabel(item)}
        </option>
      ))}
    </select>
  );
}
