import { useEffect, useState } from "react";
import { tauriApi, type Api, type Bootstrap } from "./api";
import { Home } from "./home/Home";
import { Wizard } from "./wizard/Wizard";

interface Props {
  api?: Api;
}

export default function App({ api = tauriApi }: Props) {
  const [boot, setBoot] = useState<Bootstrap | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void api
      .bootstrap()
      .then(setBoot)
      .catch((err: unknown) => setError(err instanceof Error ? err.message : "Could not open PR-Nexus"));
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
      <main className="grid min-h-screen place-items-center bg-slate-950 text-slate-300">
        <p>Loading PR-Nexus…</p>
      </main>
    );
  }
  if (!boot.onboarding.finished_at) {
    return <Wizard api={api} boot={boot} onFinished={() => void api.bootstrap().then(setBoot)} />;
  }
  return <Home api={api} boot={boot} onBoot={setBoot} />;
}
