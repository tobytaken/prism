import { net, protocol } from "electron";
import path from "node:path";
import { pathToFileURL } from "node:url";

protocol.registerSchemesAsPrivileged([
  {
    scheme: "app",
    privileges: {
      standard: true,
      secure: true,
      supportFetchAPI: true,
      stream: true,
    },
  },
]);

export function registerAppProtocol(rendererName: string) {
  protocol.handle("app", (request) => {
    const requestUrl = new URL(request.url);
    if (requestUrl.host !== "bundle") {
      return new Response("Not Found", { status: 404 });
    }

    const rendererRoot = path.resolve(
      __dirname,
      `../renderer/${rendererName}`,
    );
    const relativePath =
      requestUrl.pathname === "/" ? "index.html" : requestUrl.pathname.replace(/^\/+/, "");
    const resolvedPath = path.resolve(rendererRoot, relativePath);

    if (!resolvedPath.startsWith(rendererRoot)) {
      return new Response("Forbidden", { status: 403 });
    }

    return net.fetch(pathToFileURL(resolvedPath).toString());
  });
}
