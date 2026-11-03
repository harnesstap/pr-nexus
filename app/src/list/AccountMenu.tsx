import { useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from "react";
import type { ProfileDto } from "../api";
import { CheckIcon, GearIcon, PlusIcon } from "../icons";

/** Up to two initials: "Acme Labs" → "AL", "work" → "W". */
export function initials(name: string): string {
  const words = name.trim().split(/[\s._-]+/).filter(Boolean);
  return (words.length > 1 ? words[0][0] + words[1][0] : (words[0] ?? "?").slice(0, 1)).toUpperCase();
}

/** The GitHub avatar of the profile's account, otherwise its initials. */
export function ProfileAvatar({ profile, size = 24 }: { profile: Pick<ProfileDto, "name" | "githubLogin">; size?: number }) {
  const [failed, setFailed] = useState(false);
  const login = profile.githubLogin;
  const box = { width: size, height: size };
  if (login && !failed) {
    return (
      <img
        src={`https://avatars.githubusercontent.com/${encodeURIComponent(login)}?s=${size * 2}`}
        alt=""
        style={box}
        className="shrink-0 rounded-full bg-slate-800"
        onError={() => setFailed(true)}
      />
    );
  }
  return (
    <span
      aria-hidden="true"
      style={box}
      className="flex shrink-0 items-center justify-center rounded-full bg-indigo-500/30 text-[10px] font-semibold text-indigo-100"
    >
      {initials(profile.name)}
    </span>
  );
}

interface Props {
  profiles: ProfileDto[];
  /** Settings is the current view: the trigger shows as selected. */
  settingsActive: boolean;
  onSwitch: (id: string) => void;
  onAdd: () => void;
  onSettings: () => void;
  onAbout: () => void;
}

const ITEM =
  "flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left text-sm text-slate-200 outline-none hover:bg-slate-800 focus-visible:bg-slate-800";

/** Bottom-left account menu: profiles, Add a profile, Settings, About. */
export function AccountMenu({ profiles, settingsActive, onSwitch, onAdd, onSettings, onAbout }: Props) {
  const [open, setOpen] = useState(false);
  const menu = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const active = profiles.find((profile) => profile.active) ?? profiles[0];

  useEffect(() => {
    if (!open) {
      return;
    }
    menu.current?.querySelector<HTMLElement>('[role="menuitemradio"][aria-checked="true"], [role="menuitem"], [role="menuitemradio"]')?.focus();
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (target && !menu.current?.contains(target) && !trigger.current?.contains(target)) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  function close(focusTrigger = true) {
    setOpen(false);
    if (focusTrigger) {
      trigger.current?.focus();
    }
  }

  function pick(action: () => void) {
    close(false);
    action();
  }

  function onMenuKey(event: ReactKeyboardEvent<HTMLDivElement>) {
    const items = Array.from(menu.current?.querySelectorAll<HTMLElement>('[role^="menuitem"]') ?? []);
    const index = items.indexOf(document.activeElement as HTMLElement);
    const move = (next: number) => {
      event.preventDefault();
      items[(next + items.length) % items.length]?.focus();
    };
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        close();
        break;
      case "ArrowDown":
        move(index + 1);
        break;
      case "ArrowUp":
        move(index - 1);
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move(items.length - 1);
        break;
      case "Tab":
        close(false);
        break;
    }
  }

  if (!active) {
    return null;
  }
  return (
    <div className="relative min-w-0 flex-1">
      <button
        ref={trigger}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Profile ${active.name}, account menu`}
        className={`flex w-full cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left hover:bg-slate-800 ${settingsActive ? "bg-slate-800 text-white" : "text-slate-200"}`}
        onClick={() => setOpen((value) => !value)}
        onKeyDown={(event) => {
          if (event.key === "ArrowUp" || event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        <ProfileAvatar profile={active} />
        <span className="min-w-0 flex-1 truncate">{active.name}</span>
      </button>
      {open && (
        <div
          ref={menu}
          role="menu"
          aria-label="Account"
          onKeyDown={onMenuKey}
          className="absolute bottom-full left-0 z-30 mb-2 w-64 rounded-lg border border-slate-700 bg-slate-900 p-1 shadow-xl"
        >
          <p className="px-2 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-wide text-slate-500">Profiles</p>
          {profiles.map((profile) => (
            <button
              key={profile.id}
              type="button"
              role="menuitemradio"
              aria-checked={profile.active}
              tabIndex={-1}
              className={ITEM}
              onClick={() => pick(() => !profile.active && onSwitch(profile.id))}
            >
              <ProfileAvatar profile={profile} size={20} />
              <span className="min-w-0 flex-1">
                <span className="block truncate">{profile.name}</span>
                {profile.githubLogin && <span className="block truncate text-xs text-slate-500">@{profile.githubLogin}</span>}
              </span>
              {profile.active && <CheckIcon aria-hidden="true" className="text-indigo-300" />}
            </button>
          ))}
          <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onAdd)}>
            <span className="flex h-5 w-5 items-center justify-center text-slate-400">
              <PlusIcon />
            </span>
            Add a profile
          </button>
          <div role="separator" className="my-1 border-t border-slate-800" />
          <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onSettings)}>
            <span className="flex h-5 w-5 items-center justify-center text-slate-400">
              <GearIcon />
            </span>
            Settings
          </button>
          <button type="button" role="menuitem" tabIndex={-1} className={ITEM} onClick={() => pick(onAbout)}>
            <span className="h-5 w-5" aria-hidden="true" />
            About &amp; what's new
          </button>
        </div>
      )}
    </div>
  );
}
