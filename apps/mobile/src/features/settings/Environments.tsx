import { EXTERNAL_THREADS_GROUP, type Environment } from "@/api";
import { useEnvironments } from "@/state/queries";
import { useTheme, type Palette } from "@/theme";
import { ListRow, Section, Text } from "@/ui";

const stateColor: Record<Environment["state"], keyof Palette> = { ok: "done", warn: "warning", down: "danger", off: "textTertiary" };

/**
 * The sources your ember reads (`sources`, on Connections) or the places it works (`workplaces`, on Advanced), each with how
 * it's doing, as the server checks them.
 */
export function Environments({ show }: { show: "sources" | "workplaces" }) {
  const page = useEnvironments();
  if (page.isPending) return <ListRow title="Looking…" />;
  if (page.isError) return <ListRow title={`Can't read them: ${page.error.message}`} destructive />;
  const items = page.data.groups.flatMap((g) => g.items);
  const sources = items.filter((e) => e.group === "connections");
  const machines = items.filter((e) => ["runners", "workers", "browser", EXTERNAL_THREADS_GROUP].includes(e.group));
  const [title, rows] = show === "sources" ? ["Data sources", sources] : ["Where your ember works", machines];
  return (
    <Section title={title} count={rows.length}>
      {rows.map((e) => (
        <EnvRow key={e.id} env={e} />
      ))}
    </Section>
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
