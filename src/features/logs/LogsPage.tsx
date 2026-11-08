import { Link } from "react-router-dom";
import { ListMagnifyingGlassIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function LogsPage() {
  return (
    <FeaturePlaceholder
      title="Logs"
      description="Bounded system and application log tails with retrieval times."
      icon={<ListMagnifyingGlassIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server to inspect recent system and Laravel logs without downloading unbounded files."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { LogsPage };
