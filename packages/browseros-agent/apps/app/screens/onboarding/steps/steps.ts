import { StepConnectApps } from './StepConnectApps'
import { StepOne } from './StepOne'
import { StepTwo } from './StepTwo'

export const steps = [
  {
    id: 1,
    name: 'About You',
    component: StepOne,
  },
  {
    id: 2,
    name: 'Connect Apps',
    component: StepConnectApps,
  },
  {
    id: 3,
    name: 'Sign In',
    component: StepTwo,
  },
]
