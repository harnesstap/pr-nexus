import { Component, type ErrorInfo, type ReactNode } from "react";
import { errorMessage } from "./api";
import { copyText } from "./list/markdown";

interface Props {
  children: ReactNode;
  /** Tests replace this; the screen calls `window.location.reload`. */
  onReload?: () => void;
}

interface State {
  error: Error | null;
  info: ErrorInfo | null;
  copied: boolean;
}

/**
 * Last line of defence for a render throw: a compact dark screen with the
 * error, Reload, and Copy details, instead of an unmounted blank window.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: null, copied: false };

  static getDerivedStateFromError(error: unknown): Partial<State> {
    return { error: error instanceof Error ? error : new Error(errorMessage(error)) };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    this.setState({ info });
    console.error("PR Nexus render error", error, info.componentStack);
  }

  private details(): string {
    const { error, info } = this.state;
    const message = error ? errorMessage(error) : "Something went wrong";
    const stack = error?.stack?.trim() ?? "";
    const component = info?.componentStack?.trim() ?? "";
    return [message, stack, component ? `Component stack:\n${component}` : ""].filter((part) => part.length > 0).join("\n\n");
  }

  private copy = () => {
    void copyText(this.details()).then(
      () => {
        this.setState({ copied: true });
        window.setTimeout(() => this.setState({ copied: false }), 1500);
      },
      () => undefined,
    );
  };

  render() {
    const { error, copied } = this.state;
    if (!error) {
      return this.props.children;
    }
    const reload = this.props.onReload ?? (() => window.location.reload());
    return (
      <main role="alert" className="grid min-h-screen place-items-center bg-slate-950 px-6 text-slate-100">
        <div className="max-w-md space-y-4">
          <h1 className="text-lg font-semibold">Something went wrong</h1>
          <p className="text-sm text-rose-200">{errorMessage(error)}</p>
          <div className="flex flex-wrap gap-2">
            <button type="button" className="rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800" onClick={reload}>
              Reload
            </button>
            <button type="button" className="rounded border border-slate-700 px-3 py-1.5 text-sm hover:bg-slate-800" onClick={this.copy}>
              {copied ? "Copied" : "Copy details"}
            </button>
          </div>
        </div>
      </main>
    );
  }
}
