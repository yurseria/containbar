import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  ContainerGroup,
  ImageInfo,
  VolumeInfo,
  NetworkInfo,
} from "../types";

export function useDocker() {
  const [containers, setContainers] = useState<ContainerGroup[]>([]);
  const [images, setImages] = useState<ImageInfo[]>([]);
  const [volumes, setVolumes] = useState<VolumeInfo[]>([]);
  const [networks, setNetworks] = useState<NetworkInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [connected, setConnected] = useState(true);

  const ping = useCallback(async () => {
    try {
      await invoke("docker_ping");
      setConnected(true);
      return true;
    } catch {
      setConnected(false);
      return false;
    }
  }, []);

  const fetchContainers = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<ContainerGroup[]>("list_containers");
      setContainers(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchImages = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<ImageInfo[]>("list_images");
      setImages(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchVolumes = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<VolumeInfo[]>("list_volumes");
      setVolumes(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchNetworks = useCallback(async () => {
    setLoading(true);
    try {
      const data = await invoke<NetworkInfo[]>("list_networks");
      setNetworks(data);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const withError = useCallback(
    async (fn: () => Promise<void>) => {
      try {
        await fn();
      } catch (e) {
        setError(String(e));
        throw e;
      }
    },
    []
  );

  const startContainer = useCallback(
    async (id: string) => {
      await withError(async () => {
        await invoke("start_container", { id });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const stopContainer = useCallback(
    async (id: string) => {
      await withError(async () => {
        await invoke("stop_container", { id });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const restartContainer = useCallback(
    async (id: string) => {
      await withError(async () => {
        await invoke("restart_container", { id });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const startContainerGroup = useCallback(
    async (ids: string[]) => {
      await withError(async () => {
        await invoke("start_container_group", { ids });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const stopContainerGroup = useCallback(
    async (ids: string[]) => {
      await withError(async () => {
        await invoke("stop_container_group", { ids });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const removeContainer = useCallback(
    async (id: string, force?: boolean) => {
      await withError(async () => {
        await invoke("remove_container", { id, force: force ?? false });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const removeImage = useCallback(
    async (id: string) => {
      await withError(async () => {
        await invoke("remove_image", { id });
        await fetchImages();
      });
    },
    [fetchImages, withError]
  );

  const removeVolume = useCallback(
    async (name: string) => {
      await withError(async () => {
        await invoke("remove_volume", { name });
        await fetchVolumes();
      });
    },
    [fetchVolumes, withError]
  );

  const removeNetwork = useCallback(
    async (id: string) => {
      await withError(async () => {
        await invoke("remove_network", { id });
        await fetchNetworks();
      });
    },
    [fetchNetworks, withError]
  );

  const pullImage = useCallback(
    async (image: string) => {
      await withError(async () => {
        await invoke("pull_image", { image });
        await fetchImages();
      });
    },
    [fetchImages, withError]
  );

  const createContainer = useCallback(
    async (input: {
      name?: string;
      image: string;
      ports: { host: string; container: string }[];
      volumes: { host: string; container: string }[];
      env: string[];
      auto_start: boolean;
    }) => {
      await withError(async () => {
        await invoke("create_container", { input });
        await fetchContainers();
      });
    },
    [fetchContainers, withError]
  );

  const composeUp = useCallback(
    async (filePath: string): Promise<string> => {
      try {
        const result = await invoke<string>("compose_up", { filePath });
        await fetchContainers();
        return result;
      } catch (e) {
        setError(String(e));
        throw e;
      }
    },
    [fetchContainers]
  );

  const getContainerEnv = useCallback(
    async (id: string): Promise<string[]> => {
      return invoke<string[]>("get_container_env", { id });
    },
    []
  );

  const getContainerLogs = useCallback(
    async (id: string, tail?: string): Promise<string[]> => {
      return invoke<string[]>("get_container_logs", { id, tail });
    },
    []
  );

  return {
    containers,
    images,
    volumes,
    networks,
    loading,
    error,
    connected,
    disconnect: useCallback(() => setConnected(false), []),
    ping,
    fetchContainers,
    fetchImages,
    fetchVolumes,
    fetchNetworks,
    startContainer,
    stopContainer,
    restartContainer,
    startContainerGroup,
    stopContainerGroup,
    removeContainer,
    removeImage,
    removeVolume,
    removeNetwork,
    pullImage,
    createContainer,
    composeUp,
    getContainerEnv,
    getContainerLogs,
  };
}
