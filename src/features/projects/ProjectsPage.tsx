import { Link } from "react-router-dom";
import { FolderSimpleIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function ProjectsPage() {
  return (
    <FeaturePlaceholder
      title="Projects"
      description="Detected Laravel projects and their working directories."
      icon={<FolderSimpleIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server before browsing or detecting projects."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { ProjectsPage };
