import { Link } from "react-router-dom";
import { GaugeIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function ServicesPage() {
  return (
    <FeaturePlaceholder
      title="Services"
      description="Discovered systemd units and verified service actions."
      icon={<GaugeIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server to discover its actual services and inspect their state."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { ServicesPage };
