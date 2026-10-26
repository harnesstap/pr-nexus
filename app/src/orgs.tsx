/** GitHub's SSO entry point for an org; the opener only allows github.com links. */
export function ssoUrl(org: string): string {
  return `https://github.com/orgs/${encodeURIComponent(org)}/sso`;
}

export const TOKEN_SETTINGS_URL = "https://github.com/settings/tokens";

interface Props {
  org: string;
  checked: boolean;
  needsSso: boolean;
  onToggle: () => void;
  onOpen: (url: string) => void;
}

/** One org checkbox. Orgs that enforce SAML SSO the token is not authorized for get a hint. */
export function OrgOption({ org, checked, needsSso, onToggle, onOpen }: Props) {
  return (
    <div>
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={checked} onChange={onToggle} />
        {org}
        {needsSso && (
          <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 text-xs text-amber-200">
            SSO authorization needed
          </span>
        )}
      </label>
      {needsSso && (
        <p className="ml-6 mt-1 text-xs text-slate-400">
          This org enforces SAML single sign-on and your token is not authorized for it yet, so its PRs won't sync.{" "}
          <button type="button" className="text-violet-300 underline" onClick={() => onOpen(ssoUrl(org))}>
            Sign in with SSO
          </button>
          , then authorize the token under{" "}
          <button type="button" className="text-violet-300 underline" onClick={() => onOpen(TOKEN_SETTINGS_URL)}>
            GitHub token settings
          </button>{" "}
          (Configure SSO) and reconnect.
        </p>
      )}
    </div>
  );
}
