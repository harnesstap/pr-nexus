import { cloneElement, isValidElement, useEffect, useRef, useState, type ReactElement, type ReactNode, type RefObject } from "react";
import type { Api, Bootstrap, Preferences } from "../api";
import { errorMessage } from "../api";
import { ChangelogSettings } from "../changelog/ChangelogSettings";
import { AboutSettings, type AboutSection } from "./AboutSettings";
import { type AccountsSection } from "./AccountsSettings";
import { AgentsSettings, type AgentsSection } from "./AgentsSettings";
import { MappingSettings } from "./MappingSettings";
import { isNotificationPermission, type NotificationPermission } from "./notifications";
import { PreferencesForm, type PreferencesSection } from "./PreferencesForm";
import { ProfilesSettings } from "./ProfilesSettings";
import {
  SETTINGS_GROUPS,
  SETTINGS_NAV_VIEWPORT_PX,
  isSettingsSection,
  sectionAfterKey,
  settingsNavIsCompact,
  settingsSectionLabel,
  writeSettingsSection,
  type SettingsSection,
} from "./tabs";

interface Props {
  api: Api;
  boot: Bootstrap;
  onBoot: (boot: Bootstrap) => void;
  /** GitHub and Slack body, owned by the list. It receives a `section` prop. */
  accounts: ReactNode;
  tab: SettingsSection;
  onTabChange: (tab: SettingsSection) => void;
  onRerunSetup: () => void;
}

/** Settings: one section at a time, from the left rail or a narrow-width menu. */
export function SettingsScreen({ api, boot, onBoot, accounts, tab, onTabChange, onRerunSetup }: Props) {
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [prefsError, setPrefsError] = useState<string | null>(null);
  const [permission, setPermission] = useState<NotificationPermission>("prompt");
  const rootRef = useRef<HTMLDivElement>(null);
  const tabRefs = useRef<Partial<Record<SettingsSection, HTMLButtonElement | null>>>({});
  const compact = useCompactNav(rootRef);
  const prefsSection = preferencesSection(tab);
  const agents = agentSection(tab);
  const about = aboutSection(tab);

  useEffect(() => {
    writeSettingsSection(tab);
  }, [tab]);

  useEffect(() => {
    if (compact) {
      return;
    }
    tabRefs.current[tab]?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [tab, compact]);

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

  useEffect(() => {
    if (tab !== "notifications" || !api.notificationPermission) {
      return;
    }
    let cancel = false;
    api
      .notificationPermission()
      .then((value) => {
        if (!cancel && isNotificationPermission(value)) {
          setPermission(value);
        }
      })
      .catch(() => undefined);
    return () => {
      cancel = true;
    };
  }, [api, tab]);

  function select(next: SettingsSection) {
    onTabChange(next);
    tabRefs.current[next]?.focus();
  }

  const panel = (
    <div role="tabpanel" id="settings-panel" aria-labelledby={compact ? undefined : `settings-tab-${tab}`} aria-label={compact ? settingsSectionLabel(tab) : undefined} className="min-h-0 min-w-0 flex-1 overflow-y-auto">
      <div className="max-w-3xl pb-8">
        {accountsBody(accounts, tab === "github" || tab === "slack" ? tab : null)}
        {tab === "profiles" && <ProfilesSettings api={api} boot={boot} onBoot={onBoot} />}
        {tab === "mappings" && (
          <MappingSettings
            api={api}
            skippedInSetup={boot.onboarding.mapping_skipped}
            workspaces={boot.slack_workspaces}
            onConnectSlack={() => onTabChange("slack")}
            onCountChange={(count) => {
              if (count !== boot.mapping_count) {
                onBoot({ ...boot, mapping_count: count });
              }
            }}
          />
        )}
        {prefs && (
          <PreferencesForm
            api={api}
            prefs={prefs}
            section={prefsSection}
            permission={permission}
            onRequestPermission={() => {
              if (!api.requestNotificationPermission) {
                return;
              }
              void api
                .requestNotificationPermission()
                .then((value) => {
                  if (isNotificationPermission(value)) {
                    setPermission(value);
                  }
                })
                .catch(() => undefined);
            }}
            onSaved={(saved) => {
              setPrefs(saved);
              if (saved.bump_broadcast !== boot.bump_broadcast) {
                onBoot({ ...boot, bump_broadcast: saved.bump_broadcast });
              }
            }}
          />
        )}
        {prefsSection && prefsError && (
          <p role="alert" className="text-sm text-rose-300">
            Could not load settings: {prefsError}
          </p>
        )}
        {prefsSection && !prefs && !prefsError && <p className="text-sm text-slate-400">Loading…</p>}
        {agents && <AgentsSettings api={api} section={agents} onOpenSection={onTabChange} />}
        {tab === "changelog" && <ChangelogSettings api={api} />}
        {about && <AboutSettings api={api} onRerunSetup={onRerunSetup} section={about} />}
      </div>
    </div>
  );

  return (
    <div ref={rootRef} className={`flex h-full min-h-0 flex-1 ${compact ? "flex-col" : "gap-6"}`}>
      {compact ? (
        <label className="mb-4 block max-w-xs text-sm">
          <span className="mb-1 block text-xs font-medium uppercase tracking-wide text-slate-500">Section</span>
          <select
            id="settings-section-menu"
            aria-label="Settings section"
            className="w-full rounded border border-slate-700 bg-slate-900 px-2 py-1.5"
            value={tab}
            onChange={(event) => {
              const next = event.target.value;
              if (isSettingsSection(next)) {
                onTabChange(next);
              }
            }}
          >
            {SETTINGS_GROUPS.map((group) => (
              <optgroup key={group.id} label={group.label}>
                {group.sections.map((section) => (
                  <option key={section.id} value={section.id}>
                    {section.label}
                  </option>
                ))}
              </optgroup>
            ))}
          </select>
        </label>
      ) : (
        <nav aria-label="Settings" className="flex w-[180px] shrink-0 flex-col gap-4 overflow-y-auto border-r border-slate-800 pr-3">
          {SETTINGS_GROUPS.map((group) => (
            <div key={group.id}>
              <h2 id={`settings-group-${group.id}`} className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-slate-500">
                {group.label}
              </h2>
              <div role="tablist" aria-orientation="vertical" aria-labelledby={`settings-group-${group.id}`} className="flex flex-col gap-0.5">
                {group.sections.map((section) => (
                  <button
                    key={section.id}
                    ref={(node) => {
                      tabRefs.current[section.id] = node;
                    }}
                    type="button"
                    role="tab"
                    id={`settings-tab-${section.id}`}
                    aria-selected={tab === section.id}
                    aria-controls="settings-panel"
                    tabIndex={tab === section.id ? 0 : -1}
                    className={`rounded-md px-2 py-1.5 text-left text-sm focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-400 ${
                      tab === section.id ? "bg-violet-500/20 text-violet-100" : "text-slate-400 hover:bg-slate-900 hover:text-slate-200"
                    }`}
                    onClick={() => onTabChange(section.id)}
                    onKeyDown={(event) => {
                      const next = sectionAfterKey(tab, event.key);
                      if (next) {
                        event.preventDefault();
                        select(next);
                      }
                    }}
                  >
                    {section.label}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </nav>
      )}
      {panel}
    </div>
  );
}

function accountsBody(accounts: ReactNode, section: AccountsSection | null): ReactNode {
  if (isValidElement(accounts) && typeof accounts.type === "function") {
    return cloneElement(accounts as ReactElement<{ section?: AccountsSection | null }>, { section });
  }
  return section === "github" ? accounts : null;
}

function preferencesSection(tab: SettingsSection): PreferencesSection | null {
  if (tab === "requests" || tab === "rules" || tab === "schedule" || tab === "notifications") {
    return tab;
  }
  return null;
}

function agentSection(tab: SettingsSection): AgentsSection | null {
  if (tab === "server" || tab === "permissions" || tab === "client" || tab === "proposals" || tab === "activity") {
    return tab;
  }
  return null;
}

function aboutSection(tab: SettingsSection): AboutSection | null {
  if (tab === "app" || tab === "keychain" || tab === "setup") {
    return tab;
  }
  return null;
}

/** The rail collapses under the window minimum, or when this pane is too narrow. */
function useCompactNav(rootRef: RefObject<HTMLElement | null>): boolean {
  const [compact, setCompact] = useState(false);
  useEffect(() => {
    const root = rootRef.current;
    if (!root) {
      return;
    }
    const query = typeof window.matchMedia === "function" ? window.matchMedia(`(max-width: ${SETTINGS_NAV_VIEWPORT_PX - 1}px)`) : null;
    const update = () => {
      const viewportWidth = query ? (query.matches ? SETTINGS_NAV_VIEWPORT_PX - 1 : SETTINGS_NAV_VIEWPORT_PX) : 0;
      setCompact(settingsNavIsCompact(viewportWidth, root.clientWidth));
    };
    update();
    const observer = typeof ResizeObserver !== "undefined" ? new ResizeObserver(update) : null;
    observer?.observe(root);
    query?.addEventListener("change", update);
    return () => {
      observer?.disconnect();
      query?.removeEventListener("change", update);
    };
  }, [rootRef]);
  return compact;
}
