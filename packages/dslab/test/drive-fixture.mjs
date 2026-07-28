#!/usr/bin/env node
if (process.argv[2] === 'drive' && process.argv[3] === '--help') {
   process.stdout.write('usage: slab drive [FILE]\n');
   process.exit(0);
}

let buffered = '';

process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
   buffered += chunk;
   let newline = buffered.indexOf('\n');
   while (newline >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (line.length > 0) {
         const request = JSON.parse(line);
         if (request.method === 'clock.advance') {
            process.stdout.write(
               `${JSON.stringify({ id: request.id, result: { t: request.params.ms } })}\n`,
            );
         } else if (request.method === 'protocol.quit') {
            process.stdout.write(
               `${JSON.stringify({ id: request.id, result: { ok: true } })}\n`,
               () => process.exit(0),
            );
         }
      }
      newline = buffered.indexOf('\n');
   }
});
