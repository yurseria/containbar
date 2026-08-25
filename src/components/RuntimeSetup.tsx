import { useState } from "react";
import type { Provider, RuntimeOverview, RuntimeProviderStatus } from "../types";

const RUNTIME_META: Record<Provider, {
  name: string;
  icon: string;
  description: string;
  features: string[];
}> = {
  docker: {
    name: "Docker",
    icon: "ri-docker-line",
    description: "Use an existing Docker Desktop or OrbStack engine.",
    features: ["Existing containers", "Docker Compose"],
  },
  colima: {
    name: "Colima",
    icon: "ri-box-3-line",
    description: "A lightweight Docker-compatible VM managed by Docker Tray.",
    features: ["Docker Compose", "VM controls"],
  },
  apple: {
    name: "Apple Container",
    icon: "ri-apple-line",
    description: "Apple’s native container runtime for supported Macs.",
    features: ["Native runtime", "Compose via Mocker"],
  },
};

function actionLabel(status: RuntimeProviderStatus, selected: boolean, compact = false) {
  if (selected && status.running) return "Active";
  if (status.provider === "docker" && !status.running) return compact ? "Unavailable" : "Start Docker first";
  if (status.running) return "Switch";
  if (!status.installed) return compact ? "Install" : "Install & use";
  return compact ? "Start" : "Start & use";
}

function RuntimeCard({
  status,
  selected,
  recommended,
  busy,
  disabled,
  compact,
  onSelect,
}: {
  status: RuntimeProviderStatus;
  selected: boolean;
  recommended: boolean;
  busy: boolean;
  disabled: boolean;
  compact: boolean;
  onSelect: (provider: Provider) => Promise<void>;
}) {
  const [tooltipDismissed, setTooltipDismissed] = useState(false);
  const meta = RUNTIME_META[status.provider];
  const descriptionId = `runtime-${status.provider}-description`;
  const label = actionLabel(status, selected, compact);
  const stateLabel = status.running
    ? "Running"
    : status.installed
      ? "Stopped"
      : "Not installed";

  return (
    <article
      className={`runtime-card${selected ? " is-selected" : ""}${disabled ? " is-disabled" : ""}`}
      aria-current={selected ? "true" : undefined}
      data-tooltip-dismissed={compact && tooltipDismissed ? "true" : undefined}
      onMouseLeave={compact ? () => setTooltipDismissed(false) : undefined}
      onKeyDown={compact ? (event) => {
        if (event.key === "Escape") setTooltipDismissed(true);
      } : undefined}
    >
      <div className="runtime-card-main">
        <div className="runtime-card-icon" aria-hidden="true">
          <i className={meta.icon} />
        </div>
        <div className="runtime-card-copy">
          <div className="runtime-card-title-row">
            <h3>{meta.name}</h3>
            {recommended && <span className="runtime-recommended">Recommended</span>}
          </div>
          <p
            id={descriptionId}
            className="runtime-description"
            role={compact ? "tooltip" : undefined}
          >
            {meta.description}
          </p>
        </div>
      </div>

      <div className="runtime-state-line">
        <span className={`runtime-state runtime-state--${status.running ? "running" : "idle"}`}>
          <i className={status.running ? "ri-checkbox-circle-fill" : "ri-circle-line"} />
          {stateLabel}
        </span>
        <span className="runtime-detail">{status.detail}</span>
      </div>

      <div className="runtime-card-footer">
        <div className="runtime-features" aria-label={`${meta.name} capabilities`}>
          {meta.features.map((feature) => <span key={feature}>{feature}</span>)}
        </div>
        <button
          type="button"
          className={`runtime-action${recommended && !selected ? " runtime-action--primary" : ""}`}
          disabled={disabled || busy || (selected && status.running)}
          data-state={busy ? "loading" : selected && status.running ? "success" : "default"}
          aria-describedby={descriptionId}
          aria-label={`${label} ${meta.name}`}
          onFocus={compact ? () => setTooltipDismissed(false) : undefined}
          onClick={() => { void onSelect(status.provider).catch(() => {}); }}
        >
          {busy && <span className="runtime-action-spinner" aria-hidden="true" />}
          {label}
        </button>
      </div>
    </article>
  );
}

export function RuntimePicker({
  overview,
  busyProvider,
  error,
  compact = false,
  onSelect,
}: {
  overview: RuntimeOverview;
  busyProvider: Provider | null;
  error: string | null;
  compact?: boolean;
  onSelect: (provider: Provider) => Promise<void>;
}) {
  const recommended = overview.recommended;

  return (
    <div className={`runtime-picker${compact ? " runtime-picker--compact" : ""}`}>
      <div className="runtime-list">
        {overview.providers.map((status) => {
          const unavailableDocker = status.provider === "docker" && !status.running;
          const disabled = !status.compatible || unavailableDocker;
          return (
            <RuntimeCard
              key={status.provider}
              status={status}
              selected={overview.setup_complete && overview.selected === status.provider}
              recommended={recommended === status.provider}
              busy={busyProvider === status.provider}
              disabled={disabled}
              compact={compact}
              onSelect={onSelect}
            />
          );
        })}
      </div>
      {error && (
        <div className="runtime-setup-error" role="alert">
          <i className="ri-error-warning-line" aria-hidden="true" />
          <span>{error}</span>
        </div>
      )}
      {busyProvider && (
        <p className="runtime-progress" role="status" aria-live="polite">
          Installing or starting {RUNTIME_META[busyProvider].name}. First setup can take a few minutes.
        </p>
      )}
    </div>
  );
}

export function RuntimeSetup({
  overview,
  busyProvider,
  error,
  onSelect,
}: {
  overview: RuntimeOverview;
  busyProvider: Provider | null;
  error: string | null;
  onSelect: (provider: Provider) => Promise<void>;
}) {
  return (
    <main className="runtime-setup">
      <header className="runtime-setup-header">
        <div className="runtime-setup-mark" aria-hidden="true">
          <i className="ri-instance-line" />
        </div>
        <div>
          <h1>Choose your container runtime</h1>
          <p>Use an existing engine or let Docker Tray set one up. You can switch later without moving or deleting containers.</p>
        </div>
      </header>
      <RuntimePicker
        overview={overview}
        busyProvider={busyProvider}
        error={error}
        onSelect={onSelect}
      />
      <p className="runtime-setup-note">
        Each runtime keeps its own containers and images. Switching changes the environment you’re viewing.
      </p>
    </main>
  );
}
