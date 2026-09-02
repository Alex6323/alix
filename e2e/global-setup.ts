// Prepares the scratch decks dirs the servers run against. See
// prepare-fixtures.cjs for why this same step also runs from each webServer.
export default async function globalSetup(): Promise<void> {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const { prepareFixtures, CLIENTS } = require("./prepare-fixtures.cjs");
  for (const client of CLIENTS) prepareFixtures(client);
}
