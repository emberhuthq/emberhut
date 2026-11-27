import { EXTERNAL_THREADS_GROUP, type Environment } from "@/api";
import { useEnvironments } from "@/state/queries";
import { useTheme, type Palette } from "@/theme";
import { ListRow, Section, Text } from "@/ui";

const stateColor: Record<Environment["state"], keyof Palette> = { ok: "done", warn: "warning", down: "danger", off: "textTertiary" };

/** The sources Emberhut reads and the machines it works on, each with how it's doing, as the server checks them. */
export function Environments() {
  const page = useEnvironments();
  if (page.isPending) return <ListRow title="Looking at Emberhut's sources…" />;
  if (page.isError) return <ListRow title={`Can't read Emberhut's sources: ${page.error.message}`} destructive />;
  const items = page.data.groups.flatMap((g) => g.items);
  const sources = items.filter((e) => e.group === "connections");
  const machines = items.filter((e) => ["runners", "workers", "browser", EXTERNAL_THREADS_GROUP].includes(e.group));
  return (
    <>
      <Section title="Data sources" count={sources.length}>
        {sources.map((e) => (
          <EnvRow key={e.id} env={e} />
        ))}
      </Section>
      <Section title="Where Emberhut works" count={machines.length}>
        {machines.map((e) => (
          <EnvRow key={e.id} env={e} />
        ))}
      </Section>
    </>
  );
}

function EnvRow({ env }: { env: Environment }) {
  const { colors } = useTheme();
  return (
    <ListRow
      title={env.name}
      subtitle={[env.status, env.detail].filter(Boolean).join(" · ")}
      trailing={
        <Text style={{ color: colors[stateColor[env.state]], fontSize: 18 }} accessibilityLabel={env.state}>
          ●
        </Text>
      }
    />
  );
}
