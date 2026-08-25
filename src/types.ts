export interface PortInfo {
  private_port: number;
  public_port: number | null;
  port_type: string;
}

export interface ContainerInfo {
  id: string;
  names: string[];
  image: string;
  state: string;
  status: string;
  ports: PortInfo[];
  created: number;
  labels: Record<string, string>;
}

export interface ContainerGroup {
  name: string;
  containers: ContainerInfo[];
}

export interface ImageInfo {
  id: string;
  repo_tags: string[];
  size: number;
  created: number;
}

export interface VolumeInfo {
  name: string;
  driver: string;
  mountpoint: string;
  labels: Record<string, string>;
}

export interface NetworkInfo {
  id: string;
  name: string;
  driver: string;
  scope: string;
  containers: number;
}

/** Active container runtime provider (mirrors the Rust `ProviderKind`). */
export type Provider = "docker" | "apple" | "colima";

export interface RuntimeProviderStatus {
  provider: Provider;
  installed: boolean;
  running: boolean;
  compatible: boolean;
  detail: string;
}

export interface RuntimeOverview {
  setup_complete: boolean;
  selected: Provider;
  recommended: Provider;
  providers: RuntimeProviderStatus[];
}

/** Whether a feature is supported by the current provider. */
export interface ProviderCapabilities {
  compose: boolean;
  logTimestamps: boolean;
  /** VM resource controls (CPU/memory/disk) only exist for the Colima VM. */
  vmResources: boolean;
}

export function providerCapabilities(p: Provider): ProviderCapabilities {
  if (p === "apple") {
    return { compose: true, logTimestamps: false, vmResources: false };
  }
  // Docker (external) has no in-app VM; Colima does.
  return { compose: true, logTimestamps: true, vmResources: p === "colima" };
}

export type Tab = "containers" | "images" | "volumes" | "networks";
