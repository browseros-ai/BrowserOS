// Upload a local file and an in-memory payload into a file input, as Playwright does.
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

await input.setInputFiles({
  name: 'note.txt',
  mimeType: 'text/plain',
  buffer: Buffer.from('hello from Neo'),
});
await expect(seen).toHaveText('note.txt: hello from Neo');
return { uploaded, payload: await seen.textContent() };
