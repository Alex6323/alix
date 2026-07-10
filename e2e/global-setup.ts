// Prepares the scratch decks dirs both servers run against. See
// prepare-fixtures.cjs for why this same step also runs from each webServer.
export default async function globalSetup(): Promise<void> {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const { prepareFixtures } = require("./prepare-fixtures.cjs");
  prepareFixtures("kids");
  prepareFixtures("adult");
}
