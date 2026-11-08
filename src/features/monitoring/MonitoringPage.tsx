import { Link } from "react-router-dom";
import { ChartLineIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function MonitoringPage() {
  return (
    <FeaturePlaceholder
      title="Monitoring"
      description="Snapshot-based CPU, memory, disk, and network trends."
      icon={<ChartLineIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server to collect monitoring samples with honest timestamps."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { MonitoringPage };
