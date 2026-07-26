import { spawn } from 'node:child_process'

export async function runCommand(
  command: string,
  args: string[],
  env: NodeJS.ProcessEnv,
  cwd?: string,
): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: 'inherit' })
    child.on('close', (code) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`Command failed (${command}): ${code}`))
    })
    child.on('error', reject)
  })
}
