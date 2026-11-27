import { useLayoutEffect } from "react";
import type { RootScreenProps } from "@/navigation/types";
import { useEmberhutState } from "@/state/queries";
import { ProjectWork } from "./ProjectWork";

export function ProjectScreen({ route, navigation }: RootScreenProps<"Project">) {
  const name = useEmberhutState().data?.projects?.find((p) => p.id === route.params.id)?.name;
  useLayoutEffect(() => {
    if (name) navigation.setOptions({ title: name });
  }, [name, navigation]);
  return <ProjectWork project={route.params.id} />;
}
