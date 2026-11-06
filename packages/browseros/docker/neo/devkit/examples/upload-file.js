// Upload a file from Neo's disk into a file input, as Playwright does, and read back what the
// page got. Neo's own secret files are refused: here the host token that compose.sdk-dev.yaml
// mounts, which fails script / neo_secret_upload_refused with nothing sent and no path named.
const html = `<title>Upload</title><input type="file" id="file"><p id="seen"></p>
<script>
  const input = document.getElementById('file');
  input.addEventListener('change', async () => {
    const [file] = input.files;
    document.getElementById('seen').textContent =
      file ? file.name + ': ' + (await file.text()).split('\\n')[0] : 'cleared';
  });
</script>`;
await page.goto('data:text/html,' + encodeURIComponent(html));
const input = page.locator('#file');
const seen = page.locator('#seen');

await input.setInputFiles('/etc/os-release');
await expect(seen).toContainText('os-release: ');
const uploaded = await seen.textContent();

const token = await input.setInputFiles('/run/neo-dev/host-token').then(
  () => 'uploaded',
  ({ owner, code, delivery, message }) => ({ owner, code, delivery, message }),
);
return { uploaded, token, shownAfterRefusal: await seen.textContent() };
