import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertOctagon } from "lucide-react";
import { Button } from "./Button";
import { EmptyState } from "./Feedback";

interface Props {
  /** What failed, e.g. "This tab". */
  label: string;
  /** Keeps the error message out of sight while the region is hidden (a background tab). */
  hidden?: boolean;
  children: ReactNode;
}

interface State {
  error: Error | null;
}

/** Contains a rendering error to one region (a tab) instead of blanking the whole window. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // oxlint-disable-next-line no-console -- the crash's stack, for the webview's developer tools
    console.error(`${this.props.label} crashed`, error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    if (this.props.hidden) return null;
    return (
      <EmptyState
        icon={AlertOctagon}
        title={`${this.props.label} hit an unexpected error`}
        action={
          <Button variant="primary" onClick={() => this.setState({ error: null })}>
            Try again
          </Button>
        }
      >
        {this.state.error.message}
      </EmptyState>
    );
  }
}
