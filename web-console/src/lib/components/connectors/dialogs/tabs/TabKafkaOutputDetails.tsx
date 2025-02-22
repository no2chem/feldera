import { Control, Controller, FieldErrors } from 'react-hook-form'

import FormControl from '@mui/material/FormControl'
import FormHelperText from '@mui/material/FormHelperText'
import Grid from '@mui/material/Grid'
import TextField from '@mui/material/TextField'

import { KafkaOutputSchema } from '../KafkaOutputConnector'

const TabkafkaOutputDetails = (props: {
  control: Control<KafkaOutputSchema>
  errors: Partial<FieldErrors<KafkaOutputSchema>>
}) => {
  return (
    <Grid container spacing={6}>
      <Grid item xs={12}>
        <FormControl>
          <Controller
            name='host'
            control={props.control}
            render={({ field }) => (
              <TextField
                fullWidth
                label='Bootstrap Server Hostname'
                placeholder='kafka.example.com'
                error={Boolean(props.errors.host)}
                aria-describedby='validation-host'
                {...field}
              />
            )}
          />
          {props.errors.host && (
            <FormHelperText sx={{ color: 'error.main' }} id='validation-host'>
              {props.errors.host.message}
            </FormHelperText>
          )}
        </FormControl>
      </Grid>

      <Grid item xs={12}>
        <FormControl>
          <Controller
            name='topic'
            control={props.control}
            render={({ field }) => (
              <TextField
                fullWidth
                label='Topic Name'
                placeholder='my-topic'
                error={Boolean(props.errors.topic)}
                aria-describedby='validation-topic'
                {...field}
              />
            )}
          />
          {props.errors.topic && (
            <FormHelperText sx={{ color: 'error.main' }} id='validation-topic'>
              {props.errors.topic.message}
            </FormHelperText>
          )}
        </FormControl>
      </Grid>
    </Grid>
  )
}

export default TabkafkaOutputDetails
