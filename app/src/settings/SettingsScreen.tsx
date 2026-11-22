import { cloneElement, isValidElement, useEffect, useRef, useState, type ReactElement, type ReactNode, type RefObject } from "react";
import type { Api, Bootstrap, Preferences } from "../api";
import { errorMessage } from "../api";
import { ChangelogSettings } from "../changelog/ChangelogSettings";
import { AttentionDot } from "../list/SetupBanners";
import { AboutSettings } from "./AboutSettings";
import { type AccountsSection } from "./AccountsSettings";
import { AgentsSettings, type AgentsSection } from "./AgentsSettings";
import { MappingSettings } from "./MappingSettings";
import type { McpClient } from "./mcpClients";
import { isNotificationPermission, type NotificationPermission } from "./notifications";
import { PreferencesForm, type PreferencesSection } from "./PreferencesForm";
import { ProfilesSettings } from "./ProfilesSettings";
import {
  SETTINGS_NAV,
  SETTINGS_NAV_VIEWPORT_PX,
  isSettingsSection,
  sectionAfterKey,
  settingsAnchor,
  settingsNavIsCompact,
  settingsPage,
  settingsSectionLabel,
  writeSettingsSection,
  type SettingsPage,
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
  /** The MCP server page has a setup problem. */
  mcpAttention?: boolean;
  /** AI agent tab to show under Add to your agent. */
  mcpFocus?: McpClient | null;
  /** MCP settings or an agent config changed. */
  onSetupChanged?: () => void;
}

const ITEM =
  "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-slate-200 hover:bg-slate-800 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-violet-400";

/** Settings: one page at a time, from the left rail or a narrow-width menu. */
export function SettingsScreen({ api, boot, onBoot, accounts, tab, onTabChange, onRerunSetup, mcpAttention = false, mcpFocus = null, onSetupChanged }: Props) {
  const page = settingsPage(tab);
  const anchor = settingsAnchor(tab);
  const [prefs, setPrefs] = useState<Preferences | null>(null);
  const [prefsError, setPrefsError] = useState<string | null>(null);
  const [permission, setPermission] = useState<NotificationPermission>("prompt");
  const [anchorSeen, setAnchorSeen] = useState(anchor);
  const [changelogManual, setChangelogManual] = useState<boolean | null>(null);
  if (anchorSeen !== anchor) {
    setAnchorSeen(anchor);
    setChangelogManual(null);
  }
  const changelogOpen = changelogManual ?? anchor === "changelog";
  const rootRef = useRef<HTMLDivElement>(null);
  const compact = useCompactNav(rootRef);
  const prefsSections = preferenceSections(page);

  useEffect(() => {
    writeSettingsSection(tab);
  }, [tab]);

  useEffect(() => {
    if (compact) {
      return;
    }
    document.getElementById(`settings-tab-${page}`)?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [page, compact]);

  useEffect(() => {
    if (!anchor) {
      return;
    }
    const id = `settings-anchor-${anchor}`;
    let frames = 0;
    let raf = 0;
    const tick = () => {
      const node = document.getElementById(id);
      if (node) {
        node.scrollIntoView?.({ block: "start" });
        return;
      }
      if (frames < 20) {
        frames += 1;
        raf = requestAnimationFrame(tick);
      }
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [anchor, tab]);

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
    if (page !== "sync" || !api.notificationPermission) {
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
  }, [api, page]);

  function select(next: SettingsPage) {
    onTabChange(next);
    document.getElementById(`settings-tab-${next}`)?.focus();
  }

  function openAgentSection(section: AgentsSection) {
    onTabChange(section);
  }

  const panel = (
    <div role="tabpanel" id="settings-panel" aria-labelledby={compact ? undefined : `settings-tab-${page}`} aria-label={compact ? settingsSectionLabel(tab) : undefined} className="min-h-0 min-w-0 flex-1 overflow-y-auto">
      <div className="max-w-3xl space-y-8 pb-8">
        {accountsBody(accounts, page === "github" || page === "slack" ? page : null)}
        {page === "profiles" && <ProfilesSettings api={api} boot={boot} onBoot={onBoot} />}
        {prefs && (
          <PreferencesForm
            api={api}
            prefs={prefs}
            sections={prefsSections}
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
        {prefsSections.length > 0 && prefsError && (
          <p role="alert" className="text-sm text-rose-300">
            Could not load settings: {prefsError}
          </p>
        )}
        {prefsSections.length > 0 && !prefs && !prefsError && <p className="text-sm text-slate-400">Loading…</p>}
        {page === "review" && (
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
        {page === "mcp" && <AgentsSettings api={api} view="mcp" focusClient={mcpFocus} onOpenSection={openAgentSection} onChanged={onSetupChanged} />}
        {page === "agent-activity" && <AgentsSettings api={api} view="activity" onOpenSection={openAgentSection} onChanged={onSetupChanged} />}
        {page === "about" && (
          <>
            <AboutSettings api={api} onRerunSetup={onRerunSetup} />
            <section id="settings-anchor-changelog" className="scroll-mt-6">
              <details
                open={changelogOpen}
                onToggle={(event) => setChangelogManual(event.currentTarget.open)}
              >
                <summary className="cursor-pointer text-base font-semibold text-slate-100">Changelog</summary>
                <div className="mt-3">
                  <ChangelogSettings api={api} />
                </div>
              </details>
            </section>
          </>
        )}
      </div>
    </div>
  );

  return (
    <div ref={rootRef} className={`flex h-full min-h-0 flex-1 ${compact ? "flex-col" : "gap-6"}`}>
      {compact ? (
        <label className="mb-4 block max-w-xs text-sm">
          <span className="mb-1 block text-[10px] font-medium uppercase tracking-[0.16em] text-slate-600">Section</span>
          <select
            id="settings-section-menu"
            aria-label="Settings section"
            className="w-full rounded border border-slate-700 bg-slate-900 px-2 py-1.5"
            value={page}
            onChange={(event) => {
              const next = event.target.value;
              if (isSettingsSection(next)) {
                onTabChange(next);
              }
            }}
          >
            {SETTINGS_NAV.map((entry) =>
              entry.kind === "group" ? (
                <optgroup key={entry.id} label={entry.label}>
                  {entry.pages.map((item) => (
                    <option key={item.id} value={item.id}>
                      {item.label}
                    </option>
                  ))}
                </optgroup>
              ) : (
                <option key={entry.id} value={entry.id}>
                  {entry.label}
                </option>
              ),
            )}
          </select>
        </label>
      ) : (
        <nav aria-label="Settings" className="flex w-[180px] shrink-0 flex-col overflow-y-auto border-r border-slate-800 pr-3">
          {SETTINGS_NAV.map((entry, index) => {
            const spaced = index === 0 ? "" : "mt-5";
            if (entry.kind === "group") {
              const headingId = `settings-group-${entry.id}`;
              return (
                <div key={entry.id} role="group" aria-labelledby={headingId} className={spaced}>
                  <h2 id={headingId} className="px-2 pb-1 text-[10px] font-medium uppercase tracking-[0.16em] text-slate-600">
                    {entry.label}
                  </h2>
                  <div role="tablist" aria-orientation="vertical" aria-labelledby={headingId} className="ml-2 flex flex-col gap-0.5 border-l border-slate-700 pl-1.5">
                    {entry.pages.map((item) => (
                      <PageTab key={item.id} item={item} page={page} attention={item.id === "mcp" && mcpAttention} onTabChange={onTabChange} select={select} />
                    ))}
                  </div>
                </div>
              );
            }
            return (
              <div key={entry.id} role="tablist" aria-orientation="vertical" aria-label={entry.label} className={spaced}>
                <PageTab item={entry} page={page} attention={false} onTabChange={onTabChange} select={select} />
              </div>
            );
          })}
        </nav>
      )}
      {panel}
    </div>
  );
}

function PageTab({
  item,
  page,
  attention,
  onTabChange,
  select,
}: {
  item: { id: SettingsPage; label: string };
  page: SettingsPage;
  attention: boolean;
  onTabChange: (tab: SettingsSection) => void;
  select: (next: SettingsPage) => void;
}) {
  const selected = page === item.id;
  return (
    <button
      type="button"
      role="tab"
      id={`settings-tab-${item.id}`}
      aria-selected={selected}
      aria-controls="settings-panel"
      tabIndex={selected ? 0 : -1}
      className={`${ITEM} ${selected ? "bg-violet-500/20 text-violet-100" : ""}`}
      onClick={() => onTabChange(item.id)}
      onKeyDown={(event) => {
        const next = sectionAfterKey(page, event.key);
        if (next) {
          event.preventDefault();
          select(next);
        }
      }}
    >
      <span className="min-w-0 flex-1">{item.label}</span>
      {attention && <AttentionDot />}
    </button>
  );
}

function preferenceSections(page: SettingsPage): PreferencesSection[] {
  if (page === "review") {
    return ["requests", "rules"];
  }
  if (page === "sync") {
    return ["schedule", "notifications"];
  }
  return [];
}

function accountsBody(accounts: ReactNode, section: AccountsSection | null): ReactNode {
  if (isValidElement(accounts) && typeof accounts.type === "function") {
    return cloneElement(accounts as ReactElement<{ section?: AccountsSection | null }>, { section });
  }
  return section === "github" ? accounts : null;
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
