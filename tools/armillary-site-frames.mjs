// The site's frames for crates/romacs-vm/tests/armillary_site.rs: evaluates
// the mesh, stars and B() of thanosapollo.org's ascii-home.js (the owner's;
// everything before its DOM code) for each view on stdin, a JSON line of
// B()'s options or a {"view": ...} line, and prints {"view", "frame"} lines.
//   node tools/armillary-site-frames.mjs ascii-home.js < views.jsonl
import fs from 'node:fs';
const source = fs.readFileSync(process.argv[2], 'utf8');
const B = new Function(source.slice(source.indexOf('var K='), source.indexOf('var X=document')) + '; return B;')();
for (const line of fs.readFileSync(0, 'utf8').split('\n').filter(Boolean)) {
  const parsed = JSON.parse(line);
  const view = parsed.view ?? parsed;
  process.stdout.write(JSON.stringify({view, frame: B(view)}) + '\n');
}
