import type { NextConfig } from "next";
import path from "node:path";

const nextConfig: NextConfig = {
  // The Docker image copies .next/standalone alone, so the runtime is node plus
  // the traced files rather than the whole node_modules tree.
  output: "standalone",
  // The bot's package.json sits one level up and Next would otherwise treat
  // the repo as a workspace and nest the standalone output under web/.
  outputFileTracingRoot: path.join(__dirname),
};

export default nextConfig;
