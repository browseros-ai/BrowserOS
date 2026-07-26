import { graphql } from '@/generated/graphql/gql'

export const GetScheduledJobsByProfileIdDocument = graphql(`
  query GetScheduledJobsByProfileId($profileId: String!) {
    scheduledJobs(condition: { profileId: $profileId }, first: 100) {
      nodes {
        rowId
        name
        query
        scheduleType
        scheduleTime
        scheduleInterval
        enabled
        llmProviderId
        createdAt
        updatedAt
        lastRunAt
      }
    }
  }
`)

export const CreateScheduledJobDocument = graphql(`
  mutation CreateScheduledJob($input: CreateScheduledJobInput!) {
    createScheduledJob(input: $input) {
      scheduledJob {
        rowId
      }
    }
  }
`)

export const UpdateScheduledJobDocument = graphql(`
  mutation UpdateScheduledJob($input: UpdateScheduledJobInput!) {
    updateScheduledJob(input: $input) {
      scheduledJob {
        rowId
      }
    }
  }
`)

export const DeleteScheduledJobDocument = graphql(`
  mutation DeleteScheduledJob($rowId: String!) {
    deleteScheduledJob(input: { rowId: $rowId }) {
      deletedScheduledJobId
    }
  }
`)
