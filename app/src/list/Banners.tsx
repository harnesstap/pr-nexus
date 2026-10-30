import { CloseButton } from "../CloseButton";
import { dismissMappingHint } from "./hints";

export function MappingHint({ onOpen, onDismiss }: { onOpen: () => void; onDismiss: () => void }) {
  return (
    <div className="flex flex-wrap items-center gap-3 border-b border-slate-800 bg-slate-900 px-4 py-2 text-sm text-slate-200">
      <span>No team channels mapped yet, so review requests ask for a channel each time.</span>
      <button type="button" className="text-violet-300 underline" onClick={onOpen}>
        Map team channels
      </button>
      <CloseButton
        label="Dismiss mapping hint"
        detail="You can map team channels later in Settings → Mappings"
        shortcut={null}
        className="ml-auto"
        onClick={() => {
          dismissMappingHint();
          onDismiss();
        }}
      />
    </div>
  );
}
