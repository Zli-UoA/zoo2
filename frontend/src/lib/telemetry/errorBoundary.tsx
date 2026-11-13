import { Component, type ErrorInfo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../../components/ui/button";
import { reportReactBoundaryError } from "./capture";

interface TelemetryErrorBoundaryProps {
  children: ReactNode;
}

interface TelemetryErrorBoundaryState {
  hasError: boolean;
}

function TelemetryErrorFallback() {
  const { t } = useTranslation();
  return (
    <div
      role="alert"
      className="flex min-h-screen flex-col items-center justify-center gap-4 bg-background p-6 text-center text-foreground"
    >
      <p className="text-base font-medium">{t("telemetry.unexpectedError")}</p>
      <Button type="button" onClick={() => window.location.reload()}>
        {t("auth.reload")}
      </Button>
    </div>
  );
}

/**
 * レンダリング例外を捕捉し、サニタイズ済みの `react_boundary` テレメトリを送信して
 * 再読み込み導線を表示するルート ErrorBoundary。
 */
export class TelemetryErrorBoundary extends Component<
  TelemetryErrorBoundaryProps,
  TelemetryErrorBoundaryState
> {
  override state: TelemetryErrorBoundaryState = { hasError: false };

  static getDerivedStateFromError(): TelemetryErrorBoundaryState {
    return { hasError: true };
  }

  override componentDidCatch(error: Error, _info: ErrorInfo): void {
    reportReactBoundaryError(error);
  }

  override render(): ReactNode {
    if (this.state.hasError) {
      return <TelemetryErrorFallback />;
    }
    return this.props.children;
  }
}
