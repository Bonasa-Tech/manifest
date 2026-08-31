const { Solita } = require('@metaplex-foundation/solita');
const { spawnSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const idlDir = __dirname;

async function main() {
  ['manifest', 'wrapper'].forEach((programName) => {
    const sdkDir = path.join(__dirname, '..', 'ts', 'src', programName);
    const accountsPath = path.join(sdkDir, 'accounts/*');
    const typesPath = path.join(sdkDir, 'types/*');

    console.log('Generating TypeScript SDK to %s', sdkDir);
    console.log('... accounts in %s', accountsPath);
    console.log('... types in %s', typesPath);
    // Use a previously generated idl instead of all at once in this script
    // https://github.com/metaplex-foundation/solita because we need to add args
    // to instructions after shank runs.
    const generatedIdlPath = path.join(idlDir, `${programName}.json`);

    console.log('Using IDL at %s', generatedIdlPath);
    const idl = require(generatedIdlPath);
    const gen = new Solita(idl, { formatCode: true });

    gen.renderAndWriteTo(sdkDir).then(() => {
      if (programName === 'manifest') {
        for (const accountName of [
          'BaseAtoms',
          'QuoteAtoms',
          'GlobalAtoms',
          'QuoteAtomsPerBaseAtom',
        ]) {
          const accountFile = path.join(
            sdkDir,
            'accounts',
            `${accountName}.ts`,
          );
          const beetName = `${accountName[0].toLowerCase()}${accountName.slice(1)}Beet`;
          const source = fs.readFileSync(accountFile, 'utf8');
          const deserialize = `    return ${beetName}.deserialize(buf, offset);`;
          const checked =
            `    if (offset < 0 || buf.length - offset < ${beetName}.byteSize) {\n` +
            `      throw new RangeError('${accountName} buffer is truncated');\n` +
            `    }\n${deserialize}`;
          if (!source.includes(deserialize)) {
            throw new Error(`Unable to harden ${accountFile}`);
          }
          fs.writeFileSync(accountFile, source.replace(deserialize, checked));
        }
      }
      if (programName === 'manifest') {
        // These two events are no longer emitted, see logs.rs. Solita does not
        // carry documentation from the IDL, so the note that says so is put
        // back here every time the client is regenerated.
        const historicalBanner =
          '/**\n' +
          ' * Historical. The program stopped emitting this event when batch update\n' +
          ' * stopped logging the orders it places and cancels; it is kept so that\n' +
          ' * transactions from before that can still be decoded. Nothing emits it on new\n' +
          ' * transactions.\n' +
          ' *\n' +
          ' * To learn which orders rested, read the `Program return:` line of the batch\n' +
          ' * update instruction rather than watching for this event. Fills are still\n' +
          ' * emitted as `FillLog`, and swap still emits `PlaceOrderLogV2`, which is a\n' +
          ' * different event.\n' +
          ' *\n' +
          ' * @deprecated No longer emitted by the program.\n' +
          ' */\n';
        for (const accountName of ['PlaceOrderLog', 'CancelOrderLog']) {
          const accountFile = path.join(sdkDir, 'accounts', `${accountName}.ts`);
          const source = fs.readFileSync(accountFile, 'utf8');
          const firstImport = source.indexOf('import ');
          if (firstImport < 0) {
            throw new Error(`Unable to annotate ${accountFile}`);
          }
          fs.writeFileSync(
            accountFile,
            source.slice(0, firstImport) + historicalBanner + source.slice(firstImport),
          );
        }
      }

      console.log('Running prettier on generated files...');
      spawnSync('prettier', ['--write', sdkDir, '--trailing-comma all'], {
        stdio: 'inherit',
      });
      // Fix the fact that floats are not supported by beet.
      spawnSync(
        'sed',
        ['-i', "'s/FixedSizeUint8Array/fixedSizeUint8Array(8)/g'", typesPath],
        { stdio: 'inherit', shell: true, windowsVerbatimArguments: true },
      );
      if (programName == 'manifest') {
        spawnSync(
          'sed',
          [
            '-i',
            "'s/FixedSizeUint8Array/fixedSizeUint8Array(8)/g'",
            accountsPath,
          ],
          { stdio: 'inherit', shell: true, windowsVerbatimArguments: true },
        );
      }

      spawnSync(
        'cd ../../ && yarn format',
        ['--write', ' --config package.json', '--trailing-comma'],
        { stdio: 'inherit' },
      );

      // Make sure the client has the correct fixed header size.
      spawnSync(
        "ORIGINAL_LINE=$(awk '/export const FIXED_MANIFEST_HEADER_SIZE: number = [-.0-9]+;/' client/ts/src/constants.ts); " +
          'NEW_LINE=$(echo "export const FIXED_MANIFEST_HEADER_SIZE: number = ")$(awk \'/pub const MARKET_FIXED_SIZE: usize = [-.0-9]+;/\' programs/manifest/src/state/constants.rs | tr -d -c 0-9)$(echo ";"); ' +
          'sed --debug -i "s/${ORIGINAL_LINE}/${NEW_LINE}/" client/ts/src/constants.ts',
        [],
        { stdio: 'inherit' },
      );
      spawnSync(
        "ORIGINAL_LINE=$(awk '/export const FIXED_WRAPPER_HEADER_SIZE: number = [-.0-9]+;/' client/ts/src/constants.ts); " +
          'NEW_LINE=$(echo "export const FIXED_WRAPPER_HEADER_SIZE: number = ")$(awk \'/pub const WRAPPER_FIXED_SIZE: usize = [-.0-9]+;/\' programs/wrapper/src/wrapper_state.rs | tr -d -c 0-9)$(echo ";"); ' +
          'sed --debug -i "s/${ORIGINAL_LINE}/${NEW_LINE}/" client/ts/src/constants.ts',
        [],
        { stdio: 'inherit' },
      );
    });
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
