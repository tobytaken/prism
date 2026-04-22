import { RouterProvider } from "react-router-dom";
import { Toaster } from "sonner";

import { AppProviders } from "@/app/providers";
import { appRouter } from "@/app/router";

function App() {
  return (
    <AppProviders>
      <RouterProvider router={appRouter} />
      <Toaster position="bottom-right" theme="dark" />
    </AppProviders>
  );
}

export default App;
