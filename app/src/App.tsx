import { useEffect, useState } from "react";
import { tauriApi, type Api, type Bootstrap } from "./api";
import { KeyringBanner } from "./KeyringBanner";
import { ListScreen } from "./list/ListScreen";
import { Wizard } from "./wizard/Wizard";
import { ProfileReuse } from "./wizard/ProfileReuse";

interface Props {
  api?: Api;
}

export default function App({ api = tauriApi }: Props) {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rerun, setRerun] = useState(false);
  const [adding, setAdding] = useState(false);
  /** The profile Add a profile started from: its setup can go back there. */
  const [cameFrom, setCameFrom] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);

  useEffect(() => {
    void api
      .bootstrap()
      .then(setBoot)
      .catch((err: unknown) => setError(err instanceof Error ? err.message : "Could not open PR Nexus"));
  }, [api]);

  if (error) {
    return (
      <main className="grid min-h-screen place-items-center bg-slate-950 text-rose-200">
        <p>{error}</p>
      </main>
    );
  }
  if (!boot) {
    return (
      <main className="flex min-h-screen flex-col bg-slate-950 text-slate-300">
        <KeyringBanner api={api} />
        <p className="grid flex-1 place-items-center">Loading PR Nexus…</p>
      </main>
    );
  }
  const reload = () =>
    api
      .bootstrap()
      .then(setBoot)
      .catch((err: unknown) => setError(err instanceof Error ? err.message : "Could not open PR Nexus"));
  const switchProfile = (id: string) => {
    if (!api.switchProfile) {
      return;
    }
    setSwitching(true);
    void api
      .switchProfile(id)
      .then(reload)
      .finally(() => setSwitching(false));
  };
  const profileKey = boot.profiles?.find((profile) => profile.active)?.id ?? "default";

  if (adding) {
    return (
      <ProfileReuse
        api={api}
        profiles={boot.profiles ?? []}
        onCancel={() => setAdding(false)}
        onCreated={() =>
          void reload().then(() => {
            setAdding(false);
          })
        }
      />
    );
  }
  if (!boot.onboarding.finished_at) {
    // A new profile's setup can go back to the profile it came from.
    const previous = boot.profiles?.find((profile) => !profile.active && profile.id === cameFrom);
    return (
      <Wizard
        key={profileKey}
        api={api}
        boot={boot}
        onFinished={() => void api.bootstrap().then(setBoot)}
        onCancel={previous ? () => switchProfile(previous.id) : undefined}
        cancelLabel={previous ? `Back to ${previous.name}` : undefined}
      />
    );
  }
  if (rerun) {
    const back = () =>
      void api.bootstrap().then((next) => {
        setBoot(next);
        setRerun(false);
      });
    return <Wizard api={api} boot={{ ...boot, resume_step: boot.steps[0] ?? boot.resume_step }} onFinished={back} onCancel={back} />;
  }
  return (
    <div aria-busy={switching} className={switching ? "pointer-events-none opacity-70 transition-opacity" : undefined}>
      <ListScreen
        key={profileKey}
        api={api}
        boot={boot}
        onBoot={setBoot}
        onRerunSetup={() => setRerun(true)}
        onSwitchProfile={switchProfile}
        onAddProfile={() => {
          setCameFrom(profileKey);
          setAdding(true);
        }}
      />
    </div>
  );
}
