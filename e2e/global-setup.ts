// Prepares the scratch decks dir the server will run against. See
// prepare-fixtures.cjs for why this same step also runs from webServer.command.
export default async function globalSetup(): Promise<void> {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const { prepareFixtures } = require("./prepare-fixtures.cjs");
  prepareFixtures();
}
