import { Link } from "react-router-dom";
import { FilesIcon } from "@phosphor-icons/react";
import { FeaturePlaceholder } from "@/components/shared/FeaturePlaceholder";
import { Button } from "@/components/ui/button";

function FileManagerPage() {
  return (
    <FeaturePlaceholder
      title="File Manager"
      description="Browse, upload, download, and organize remote files over SFTP."
      icon={<FilesIcon className="size-4" aria-hidden="true" />}
      emptyTitle="No server selected"
      emptyDescription="Choose a server to browse its files over the authenticated SSH connection."
      action={
        <Button asChild variant="secondary" size="sm">
          <Link to="/servers">Open servers</Link>
        </Button>
      }
    />
  );
}

export { FileManagerPage };
