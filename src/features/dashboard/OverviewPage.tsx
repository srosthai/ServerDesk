import { Link } from "react-router-dom";
import { SquaresFourIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function OverviewPage() {
  return (
    <FeaturePlaceholder
      title="Overview"
      description="Server identity, connection state, and health at a glance."
      icon={<SquaresFourIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server to see its live state, resource usage, and latest sample time."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { OverviewPage };
