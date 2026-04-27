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
    const decodedPath = decodePath(requestUrl.pathname);
    const relativePath =
      decodedPath === "/" ? "index.html" : decodedPath.replace(/^\/+/, "");
    const resolvedPath = path.resolve(rendererRoot, relativePath);

    if (!resolvedPath.startsWith(rendererRoot)) {
      return new Response("Forbidden", { status: 403 });
    }

    return net.fetch(pathToFileURL(resolvedPath).toString());
  });
}

function decodePath(pathname: string) {
  let decodedPath = pathname;

  for (let index = 0; index < 4; index += 1) {
    const nextPath = decodeURIComponent(decodedPath);
    if (nextPath === decodedPath) {
      return decodedPath;
    }
    decodedPath = nextPath;
  }

  return decodedPath;
}
