import { useLayoutEffect } from "react";
import type { RootScreenProps } from "@/navigation/types";
import { useEmberhutState } from "@/state/queries";
import { EmptyState, Loading } from "@/ui";
import { ProjectWork } from "./ProjectWork";

/** One area of a project: its questions, its tree and its findings. */
export function AreaScreen({ route, navigation }: RootScreenProps<"Area">) {
  const state = useEmberhutState();
  const area = state.data?.areas?.find((a) => a.id === route.params.id);
  useLayoutEffect(() => {
    if (area) navigation.setOptions({ title: area.name });
  }, [area, navigation]);
  if (!state.data) return <Loading label="Reaching Emberhut" />;
  if (!area) return <EmptyState icon="git-network-outline" title="Area not found" />;
  return <ProjectWork project={area.pid} area={area.id} />;
}
