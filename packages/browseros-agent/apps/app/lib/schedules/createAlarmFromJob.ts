import type { ScheduledJob } from './scheduleTypes'

const getNextScheduledTime = (timeString: string): number => {
  const [hours, minutes] = timeString.split(':').map(Number)
  const now = new Date()
  const scheduled = new Date()

  scheduled.setHours(hours, minutes, 0, 0)

  // If time has passed today, schedule for tomorrow
  if (scheduled.getTime() <= now.getTime()) {
    scheduled.setDate(scheduled.getDate() + 1)
  }

  return scheduled.getTime()
}

export const createAlarmFromJob = async (job: ScheduledJob) => {
  const alarmName = `scheduled-job-${job.id}`

  let time: chrome.alarms.AlarmCreateInfo | undefined

  if (job.scheduleType === 'daily' && job.scheduleTime) {
    time = {
      when: getNextScheduledTime(job.scheduleTime),
      periodInMinutes: 24 * 60, // Repeat every 24 hours
    }
  } else if (job.scheduleType === 'hourly' && job.scheduleInterval) {
    const intervalInMinutes = job.scheduleInterval * 60
    time = {
      delayInMinutes: intervalInMinutes,
      periodInMinutes: intervalInMinutes,
    }
  } else if (job.scheduleType === 'minutes' && job.scheduleInterval) {
    time = {
      delayInMinutes: job.scheduleInterval,
      periodInMinutes: job.scheduleInterval,
    }
  }
  if (time) {
    await chrome.alarms.create(alarmName, time)
  }
}
