import { useEffect, useState } from "react";
import type { Api } from "../api";
import { decideWhatsNew, lastSeenVersion, recordSeenVersion, type ChangelogRelease } from "./changelog";
import { RELEASES } from "./data";
import { WhatsNewModal } from "./WhatsNewModal";

/** Opens "What's new" once after an update; see `decideWhatsNew`. */
export function WhatsNew({ api, onFullChangelog, releases = RELEASES }: { api: Pick<Api, "appInfo">; onFullChangelog: () => void; releases?: ChangelogRelease[] }) {
  const [open, setOpen] = useState<{ current: string; releases: ChangelogRelease[] } | null>(null);
  useEffect(() => {
    let cancel = false;
    Promise.resolve()
      .then(() => api.appInfo())
      .then((info) => {
        if (cancel) {
          return;
        }
        const decision = decideWhatsNew(info.version, lastSeenVersion(), releases);
        if (decision.show) {
          setOpen({ current: info.version, releases: decision.releases });
        } else if (decision.record) {
          recordSeenVersion(decision.record);
        }
      })
      .catch(() => undefined);
    return () => {
      cancel = true;
    };
  }, [api, releases]);

  if (!open) {
    return null;
  }
  const dismiss = () => {
    recordSeenVersion(open.current);
    setOpen(null);
  };
  return (
    <WhatsNewModal
      current={open.current}
      releases={open.releases}
      onClose={dismiss}
      onFullChangelog={() => {
        dismiss();
        onFullChangelog();
      }}
    />
  );
}
