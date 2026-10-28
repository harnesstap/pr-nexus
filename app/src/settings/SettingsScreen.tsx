import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Api, Bootstrap, Preferences } from "../api";
import { errorMessage } from "../api";
import { AboutSettings } from "./AboutSettings";
import { AgentsSettings } from "./AgentsSettings";
import { MappingSettings } from "./MappingSettings";
import { PreferencesForm } from "./PreferencesForm";
import { SETTINGS_TABS, tabAfterKey, type SettingsTab } from "./tabs";

interface Props {
  api: Api;
  boot: Bootstrap;
  onBoot: (boot: Bootstrap) => void;
  /** Accounts tab body: the org picker and Slack settings, owned by the list. */
  accounts: ReactNode;
  tab: SettingsTab;
  onTabChange: (tab: SettingsTab) => void;
  onRerunSetup: () => void;
}

/** Settings (§8.2): Accounts, Mappings, Review, Sync, About. */
export function SettingsScreen({ api, boot, onBoot, accounts, tab, onTabChange, onRerunSetup }: Props) {
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [prefsError, setPrefsError] = useState<string | null>(null);
  const tabRefs = useRef<Partial<Record<SettingsTab, HTMLButtonElement | null>>>({});

  useEffect(() => {
    let cancel = false;
    api
      .loadPreferences()
      .then((next) => {
        if (!cancel) {
          setPrefs(next);
        }
      })
      .catch((err: unknown) => {
        if (!cancel) {
          setPrefsError(errorMessage(err));
        }
      });
    return () => {
      cancel = true;
    };
  }, [api]);

  function select(next: SettingsTab) {
    onTabChange(next);
    tabRefs.current[next]?.focus();
  }

  function prefsBody(section: "review" | "sync") {
    if (prefsError) {
      return (
        <p role="alert" className="text-sm text-rose-300">
          Could not load settings: {prefsError}
        </p>
      );
    }
    if (!prefs) {
      return <p className="text-sm text-slate-400">Loading…</p>;
    }
    return (
      <PreferencesForm
        key={section}
        api={api}
        prefs={prefs}
        section={section}
        onSaved={(saved) => {
          setPrefs(saved);
          if (saved.bump_broadcast !== boot.bump_broadcast) {
            onBoot({ ...boot, bump_broadcast: saved.bump_broadcast });
          }
        }}
      />
    );
  }

  return (
    <div className="mx-auto max-w-3xl">
      <div role="tablist" aria-label="Settings sections" className="mb-6 flex gap-1 border-b border-slate-800">
        {SETTINGS_TABS.map((item) => (
          <button
            key={item.id}
            ref={(node) => {
              tabRefs.current[item.id] = node;
            }}
            type="button"
            role="tab"
            id={`settings-tab-${item.id}`}
            aria-selected={tab === item.id}
            aria-controls={`settings-panel-${item.id}`}
            tabIndex={tab === item.id ? 0 : -1}
            className={`-mb-px border-b-2 px-3 py-2 text-sm ${
              tab === item.id ? "border-violet-400 text-slate-100" : "border-transparent text-slate-400 hover:text-slate-200"
            }`}
            onClick={() => onTabChange(item.id)}
            onKeyDown={(event) => {
              const next = tabAfterKey(tab, event.key);
              if (next) {
                event.preventDefault();
                select(next);
              }
            }}
          >
            {item.label}
          </button>
        ))}
      </div>
      <div role="tabpanel" id={`settings-panel-${tab}`} aria-labelledby={`settings-tab-${tab}`}>
        {tab === "accounts" && accounts}
        {tab === "mappings" && (
          <MappingSettings
            api={api}
            skippedInSetup={boot.onboarding.mapping_skipped}
            workspaces={boot.slack_workspaces}
            onConnectSlack={() => onTabChange("accounts")}
            onCountChange={(count) => {
              if (count !== boot.mapping_count) {
                onBoot({ ...boot, mapping_count: count });
              }
            }}
          />
        )}
        {tab === "review" && prefsBody("review")}
        {tab === "sync" && prefsBody("sync")}
        {tab === "agents" && <AgentsSettings api={api} />}
        {tab === "about" && <AboutSettings api={api} onRerunSetup={onRerunSetup} />}
      </div>
    </div>
  );
}
