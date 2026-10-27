import { useEffect, useState } from "react";
import { tauriApi, type Api, type Bootstrap } from "./api";
import { KeyringBanner } from "./KeyringBanner";
import { ListScreen } from "./list/ListScreen";
import { Wizard } from "./wizard/Wizard";

interface Props {
  api?: Api;
}

export default function App({ api = tauriApi }: Props) {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rerun, setRerun] = useState(false);

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
  if (!boot.onboarding.finished_at) {
    return <Wizard api={api} boot={boot} onFinished={() => void api.bootstrap().then(setBoot)} />;
  }
  if (rerun) {
    const back = () =>
      void api.bootstrap().then((next) => {
        setBoot(next);
        setRerun(false);
      });
    return <Wizard api={api} boot={{ ...boot, resume_step: boot.steps[0] ?? boot.resume_step }} onFinished={back} onCancel={back} />;
  }
  return <ListScreen api={api} boot={boot} onBoot={setBoot} onRerunSetup={() => setRerun(true)} />;
}
