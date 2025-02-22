// Table showing the list of pipelines for a user.
//
// The rows of the table can be expanded for even more details.

import useStatusNotification from '$lib/components/common/errors/useStatusNotification'
import { DataGridFooter } from '$lib/components/common/table/DataGridFooter'
import DataGridSearch from '$lib/components/common/table/DataGridSearch'
import DataGridToolbar from '$lib/components/common/table/DataGridToolbar'
import AnalyticsPipelineTput from '$lib/components/streaming/management/AnalyticsPipelineTput'
import { PipelineRevisionStatusChip } from '$lib/components/streaming/management/RevisionStatus'
import { ClientPipelineStatus, usePipelineStateStore } from '$lib/compositions/streaming/management/StatusContext'
import useDeletePipeline from '$lib/compositions/streaming/management/useDeletePipeline'
import usePausePipeline from '$lib/compositions/streaming/management/usePausePipeline'
import { usePipelineMetrics } from '$lib/compositions/streaming/management/usePipelineMetrics'
import useShutdownPipeline from '$lib/compositions/streaming/management/useShutdownPipeline'
import useStartPipeline from '$lib/compositions/streaming/management/useStartPipeline'
import { humanSize } from '$lib/functions/common/string'
import { tuple } from '$lib/functions/common/tuple'
import { PipelineManagerQuery } from '$lib/services/defaultQueryFn'
import {
  ApiError,
  AttachedConnector,
  ConnectorDescr,
  Pipeline,
  PipelineId,
  PipelineRevision,
  PipelinesService,
  PipelineStatus,
  Relation,
  UpdatePipelineRequest,
  UpdatePipelineResponse
} from '$lib/services/manager'
import { LS_PREFIX } from '$lib/types/localStorage'
import { format } from 'd3-format'
import dayjs from 'dayjs'
import Link from 'next/link'
import React, { useCallback, useEffect, useState } from 'react'
import CustomChip from 'src/@core/components/mui/chip'
import { match, P } from 'ts-pattern'

import { Icon } from '@iconify/react'
import { useHash, useLocalStorage } from '@mantine/hooks'
import ExpandMoreIcon from '@mui/icons-material/ExpandMore'
import { Button } from '@mui/material'
import Badge from '@mui/material/Badge'
import Box from '@mui/material/Box'
import Card from '@mui/material/Card'
import Grid from '@mui/material/Grid'
import IconButton from '@mui/material/IconButton'
import List from '@mui/material/List'
import ListItem from '@mui/material/ListItem'
import ListItemIcon from '@mui/material/ListItemIcon'
import ListItemText from '@mui/material/ListItemText'
import ListSubheader from '@mui/material/ListSubheader'
import Paper from '@mui/material/Paper'
import Tooltip from '@mui/material/Tooltip'
import {
  DataGridPro,
  DataGridProProps,
  GRID_DETAIL_PANEL_TOGGLE_COL_DEF,
  GridColDef,
  GridRenderCellParams,
  GridRowId,
  GridValueSetterParams,
  useGridApiRef
} from '@mui/x-data-grid-pro'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

interface ConnectorData {
  relation: Relation
  connections: [AttachedConnector, ConnectorDescr][]
}
type InputOrOutput = 'input' | 'output'

// Joins the relation with attached connectors and connectors and returns it as
// a list of ConnectorData that has a list of `connections` for each `relation`.
function getConnectorData(revision: PipelineRevision, direction: InputOrOutput): ConnectorData[] {
  const schema = revision.program.schema
  if (!schema) {
    // This means the backend sent invalid data,
    // revisions should always have a schema
    throw Error('Pipeline revision has no schema.')
  }

  const relations = direction === ('input' as const) ? schema.inputs : schema.outputs
  const attachedConnectors = revision.pipeline.attached_connectors
  const connectors = revision.connectors

  return relations.map(relation => {
    const connections: [AttachedConnector, ConnectorDescr][] = attachedConnectors
      .filter(ac => ac.relation_name === relation.name)
      .map(ac => {
        const connector = connectors.find(c => c.connector_id === ac?.connector_id)
        if (!connector) {
          // This can't happen in a revision
          throw Error('Attached connector has no connector.')
        }
        return [ac, connector] as [AttachedConnector, ConnectorDescr]
      })

    return { relation, connections }
  })
}

const DetailPanelContent = (props: { row: Pipeline }) => {
  const [inputs, setInputs] = useState<ConnectorData[]>([])
  const [outputs, setOutputs] = useState<ConnectorData[]>([])
  const { descriptor, state } = props.row

  const pipelineRevisionQuery = useQuery(PipelineManagerQuery.pipelineLastRevision(descriptor.pipeline_id))
  useEffect(() => {
    if (!pipelineRevisionQuery.isLoading && !pipelineRevisionQuery.isError && pipelineRevisionQuery.data) {
      setInputs(getConnectorData(pipelineRevisionQuery.data, 'input'))
      setOutputs(getConnectorData(pipelineRevisionQuery.data, 'output'))
    }
  }, [
    pipelineRevisionQuery.isLoading,
    pipelineRevisionQuery.isError,
    pipelineRevisionQuery.data,
    setInputs,
    setOutputs
  ])

  const metrics = usePipelineMetrics({
    pipelineId: descriptor.pipeline_id,
    status: state.current_status,
    refetchMs: 1000
  })

  function getRelationColumns(direction: InputOrOutput): GridColDef<ConnectorData>[] {
    return [
      {
        field: 'name',
        headerName: direction === 'input' ? 'Input' : 'Output',
        flex: 0.5,
        renderCell: params => {
          if (params.row.connections.length > 0) {
            return params.row.connections.map(c => c[1].name).join(', ')
          } else {
            return <Box sx={{ fontStyle: 'italic' }}>No connection.</Box>
          }
        }
      },
      {
        field: 'config',
        valueGetter: params => params.row.relation.name,
        headerName: direction === 'input' ? 'Table' : 'View',
        flex: 0.8
      },
      {
        field: 'records',
        headerName: 'Records',
        flex: 0.15,
        renderCell: params => {
          if (params.row.connections.length > 0) {
            const records =
              direction === 'input'
                ? metrics.input.get(params.row.relation.name)?.total_records
                : metrics.output.get(params.row.relation.name)?.transmitted_records
            return format('.1s')(records || 0)
          } else {
            // TODO: we need to count records also when relation doesn't have
            // connections in the backend.
            return '-'
          }
        }
      },
      {
        field: 'traffic',
        headerName: 'Traffic',
        flex: 0.15,
        renderCell: params => {
          const bytes =
            direction === 'input'
              ? metrics.input.get(params.row.relation.name)?.total_bytes
              : metrics.output.get(params.row.relation.name)?.transmitted_bytes
          return humanSize(bytes || 0)
        }
      },
      {
        field: 'action',
        headerName: 'Action',
        flex: 0.15,
        renderCell: params => (
          <>
            <Tooltip title={direction === 'input' ? 'Inspect Table' : 'Inspect View'}>
              <IconButton
                size='small'
                href={`/streaming/inspection/?pipeline_id=${descriptor.pipeline_id}&relation=${params.row.relation.name}`}
              >
                <Icon icon='bx:show' fontSize={20} />
              </IconButton>
            </Tooltip>
            {direction === 'input' && state.current_status == PipelineStatus.RUNNING && (
              <Tooltip title='Import Data'>
                <IconButton
                  size='small'
                  href={`/streaming/inspection/?pipeline_id=${descriptor.pipeline_id}&relation=${params.row.relation.name}&tab=insert`}
                >
                  <Icon icon='bx:upload' fontSize={20} />
                </IconButton>
              </Tooltip>
            )}
          </>
        )
      }
    ]
  }

  return !pipelineRevisionQuery.isLoading && !pipelineRevisionQuery.isError ? (
    <Box display='flex' sx={{ m: 2 }} justifyContent='center'>
      <Grid container spacing={3} sx={{ height: 1, width: '95%' }} alignItems='stretch'>
        <Grid item xs={4}>
          <Card>
            <List subheader={<ListSubheader>Configuration</ListSubheader>} dense>
              <ListItem>
                <ListItemIcon>
                  <Icon icon='fluent-emoji-high-contrast:id-button' fontSize={20} />
                </ListItemIcon>
                <ListItemText primary={pipelineRevisionQuery.data?.pipeline.pipeline_id || 'not set'} />
              </ListItem>
              <ListItem>
                <ListItemIcon>
                  <Icon icon='bi:filetype-sql' fontSize={20} />
                </ListItemIcon>
                <ListItemText primary={pipelineRevisionQuery.data?.program.name || 'not set'} />
              </ListItem>
              {state.current_status == PipelineStatus.RUNNING && (
                <>
                  <ListItem>
                    <Tooltip title='Pipeline Running Since'>
                      <ListItemIcon>
                        <Icon icon='clarity:date-line' fontSize={20} />
                      </ListItemIcon>
                    </Tooltip>
                    <ListItemText
                      primary={state.created ? dayjs(state.created).format('MM/DD/YYYY HH:MM') : 'Not running'}
                    />
                  </ListItem>
                  <ListItem>
                    <Tooltip title='Pipeline Port'>
                      <ListItemIcon>
                        <Icon icon='carbon:port-input' fontSize={20} />
                      </ListItemIcon>
                    </Tooltip>
                    <ListItemText className='pipelinePort' primary={state.location || '0000'} />
                  </ListItem>
                </>
              )}
            </List>
          </Card>
        </Grid>

        <Grid item xs={8}>
          <Paper>
            <AnalyticsPipelineTput metrics={metrics.global} />
          </Paper>
        </Grid>

        <Grid item xs={12}>
          {/* className referenced by webui-tester */}
          <Paper className='inputStats'>
            <DataGridPro
              autoHeight
              getRowId={(row: ConnectorData) => row.relation.name}
              columns={getRelationColumns('input')}
              rows={inputs}
              sx={{ flex: 1 }}
              hideFooter
            />
          </Paper>
        </Grid>

        <Grid item xs={12}>
          {/* className referenced by webui-tester */}
          <Paper className='outputStats'>
            <DataGridPro
              autoHeight
              getRowId={(row: ConnectorData) => row.relation.name}
              columns={getRelationColumns('output')}
              rows={outputs}
              sx={{ flex: 1 }}
              hideFooter
            />
          </Paper>
        </Grid>
      </Grid>
    </Box>
  ) : (
    <Box>Loading...</Box>
  )
}

const pipelineStatusToClientStatus = (status: PipelineStatus) => {
  return match(status)
    .with(PipelineStatus.SHUTDOWN, () => ClientPipelineStatus.INACTIVE)
    .with(PipelineStatus.PROVISIONING, () => ClientPipelineStatus.PROVISIONING)
    .with(PipelineStatus.INITIALIZING, () => ClientPipelineStatus.INITIALIZING)
    .with(PipelineStatus.PAUSED, () => ClientPipelineStatus.PAUSED)
    .with(PipelineStatus.RUNNING, () => ClientPipelineStatus.RUNNING)
    .with(PipelineStatus.SHUTTING_DOWN, () => ClientPipelineStatus.SHUTTING_DOWN)
    .with(PipelineStatus.FAILED, () => ClientPipelineStatus.FAILED)
    .exhaustive()
}

// Only show the details tab button if this pipeline has a revision
function CustomDetailPanelToggle(props: Pick<GridRenderCellParams, 'id' | 'value' | 'row'>) {
  const { value: isExpanded, row: row } = props
  const [hasRevision, setHasRevision] = useState<boolean>(false)

  const pipelineRevisionQuery = useQuery<PipelineRevision | null>([
    'pipelineLastRevision',
    { pipeline_id: props.row.descriptor.pipeline_id }
  ])
  useEffect(() => {
    if (!pipelineRevisionQuery.isLoading && !pipelineRevisionQuery.isError && pipelineRevisionQuery.data != null) {
      setHasRevision(true)
    }
  }, [pipelineRevisionQuery.isLoading, pipelineRevisionQuery.isError, pipelineRevisionQuery.data])

  return (isExpanded ||
    row.state.current_status === PipelineStatus.RUNNING ||
    row.state.current_status === PipelineStatus.PAUSED) &&
    hasRevision ? (
    <IconButton size='small' tabIndex={-1} aria-label={isExpanded ? 'Close' : 'Open'}>
      <ExpandMoreIcon
        sx={{
          transform: `rotateZ(${isExpanded ? 180 : 0}deg)`,
          transition: theme =>
            theme.transitions.create('transform', {
              duration: theme.transitions.duration.shortest
            })
        }}
        fontSize='inherit'
      />
    </IconButton>
  ) : (
    <></>
  )
}

export default function PipelineTable() {
  const [rows, setRows] = useState<Pipeline[]>([])
  const [filteredData, setFilteredData] = useState<Pipeline[]>([])
  const setPipelineStatus = usePipelineStateStore(state => state.setStatus)
  const [paginationModel, setPaginationModel] = useState({
    pageSize: 7,
    page: 0
  })

  const pipelineQuery = useQuery({
    ...PipelineManagerQuery.pipeline(),
    refetchInterval: 2000
  })
  const { isLoading, isError, data, error } = pipelineQuery
  useEffect(() => {
    if (!isLoading && !isError) {
      setRows(data)
      for (const { descriptor, state } of data) {
        // If we're not in the desired status, we know better what to display as
        // status in the client (something pending), so we don't reset it until
        // desired status is reached and rely on whatever we set with
        // `setPipelineStatus` in the start/stop/pause hooks.
        if (state.current_status == state.desired_status || state.current_status == PipelineStatus.FAILED) {
          setPipelineStatus(descriptor.pipeline_id, pipelineStatusToClientStatus(state.current_status))
        }
      }
    }
    if (isError) {
      throw error
    }
  }, [isLoading, isError, data, setRows, setPipelineStatus, error])

  const getDetailPanelContent = useCallback<NonNullable<DataGridProProps['getDetailPanelContent']>>(
    ({ row }) => <DetailPanelContent row={row} />,
    []
  )

  const columns: GridColDef[] = [
    {
      ...GRID_DETAIL_PANEL_TOGGLE_COL_DEF,
      renderCell: params => <CustomDetailPanelToggle id={params.id} value={params.value} row={params.row} />
    },
    {
      field: 'name',
      headerName: 'Name',
      editable: true,
      flex: 2,
      valueGetter: params => params.row.descriptor.name,
      valueSetter: (params: GridValueSetterParams) => {
        params.row.descriptor.name = params.value
        return params.row
      }
    },
    {
      field: 'description',
      headerName: 'Description',
      editable: true,
      flex: 3,
      valueGetter: params => params.row.descriptor.description,
      valueSetter: (params: GridValueSetterParams) => {
        params.row.descriptor.description = params.value
        return params.row
      }
    },
    {
      field: 'modification',
      headerName: 'Changes',
      flex: 1,
      renderCell: (params: GridRenderCellParams) => {
        return <PipelineRevisionStatusChip pipeline={params.row} />
      }
    },
    {
      field: 'status',
      headerName: 'Status',
      flex: 1,
      renderCell: PipelineStatusCell
    },
    {
      field: 'actions',
      headerName: 'Actions',
      flex: 1.0,
      renderCell: PipelineActions
    }
  ]

  // Makes sure we can edit name and description in the table
  const apiRef = useGridApiRef()
  const queryClient = useQueryClient()
  const { pushMessage } = useStatusNotification()
  const mutation = useMutation<
    UpdatePipelineResponse,
    ApiError,
    { pipeline_id: PipelineId; request: UpdatePipelineRequest }
  >(args => {
    return PipelinesService.updatePipeline(args.pipeline_id, args.request)
  })
  const onUpdateRow = (newRow: Pipeline, oldRow: Pipeline) => {
    console.log('onUpdateRow;;', {
      name: newRow.descriptor.name,
      description: newRow.descriptor.description,
      program_id: newRow.descriptor.program_id
    })
    mutation.mutate(
      {
        pipeline_id: newRow.descriptor.pipeline_id,
        request: {
          name: newRow.descriptor.name,
          description: newRow.descriptor.description,
          program_id: newRow.descriptor.program_id
        }
      },
      {
        onError: (error: ApiError) => {
          queryClient.invalidateQueries(['pipeline'])
          queryClient.invalidateQueries(['pipelineStatus', { program_id: newRow.descriptor.pipeline_id }])
          pushMessage({ message: error.body.message, key: new Date().getTime(), color: 'error' })
          apiRef.current.updateRows([oldRow])
        }
      }
    )

    return newRow
  }

  const btnAdd = (
    <Button variant='contained' size='small' href='/streaming/builder/' id='btn-add-pipeline' key='0'>
      Add pipeline
    </Button>
  )

  const [expandedRows, setExpandedRows] = useLocalStorage({
    key: LS_PREFIX + 'pipelines/expanded',
    defaultValue: [] as GridRowId[]
  })
  const [hash, setHash] = useHash()

  // Cannot initialize in useState because hash is not available during SSR
  useEffect(() => {
    const anchor = hash.slice(1)

    setExpandedRows(expandedRows =>
      (expandedRows.includes(anchor) ? expandedRows : [...expandedRows, anchor]).filter(
        row =>
          data?.find(
            p =>
              p.descriptor.pipeline_id === row &&
              [
                PipelineStatus.PROVISIONING,
                PipelineStatus.INITIALIZING,
                PipelineStatus.PAUSED,
                PipelineStatus.RUNNING,
                PipelineStatus.SHUTTING_DOWN
              ].includes(p.state.current_status)
          )
      )
    )
  }, [hash, setExpandedRows, data])

  const updateExpandedRows = (newExpandedRows: GridRowId[]) => {
    const anchor = hash.slice(1)
    if (newExpandedRows.length < expandedRows.length && !newExpandedRows.includes(anchor)) {
      setHash('')
    }
    setExpandedRows(newExpandedRows)
  }

  return (
    <Card>
      <DataGridPro
        autoHeight
        apiRef={apiRef}
        getRowId={(row: Pipeline) => row.descriptor.pipeline_id}
        columns={columns}
        rowThreshold={0}
        getDetailPanelHeight={() => 'auto'}
        getDetailPanelContent={getDetailPanelContent}
        slots={{
          toolbar: DataGridToolbar,
          footer: DataGridFooter
        }}
        rows={filteredData.length ? filteredData : rows}
        pageSizeOptions={[7, 10, 25, 50]}
        paginationModel={paginationModel}
        onPaginationModelChange={setPaginationModel}
        processRowUpdate={onUpdateRow}
        loading={isLoading}
        slotProps={{
          baseButton: {
            variant: 'outlined'
          },
          toolbar: {
            children: [
              btnAdd,
              <div style={{ marginLeft: 'auto' }} key='2' />,
              <DataGridSearch fetchRows={pipelineQuery} setFilteredData={setFilteredData} key='99' />
            ]
          },
          footer: {
            children: btnAdd
          }
        }}
        detailPanelExpandedRowIds={expandedRows}
        onDetailPanelExpandedRowIdsChange={updateExpandedRows}
      />
    </Card>
  )
}

const usePipelineState = (params: { row: Pipeline }) => {
  const pipeline = params.row.descriptor
  const pipelineStatus = usePipelineStateStore(state => state.clientStatus)
  const curProgramQuery = useQuery({
    ...PipelineManagerQuery.programCode(pipeline.program_id!),
    enabled: pipeline.program_id != null
  })

  const programReady =
    !curProgramQuery.isLoading && !curProgramQuery.isError && curProgramQuery.data.status === 'Success'
  const currentStatus = pipelineStatus.get(pipeline.pipeline_id) ?? ClientPipelineStatus.UNKNOWN
  return tuple(currentStatus, programReady)
}

const PipelineStatusCell = (params: { row: Pipeline } & GridRenderCellParams) => {
  const state = usePipelineState(params)

  const shutdownPipelineClick = useShutdownPipeline()

  const chip = match(state)
    .with([ClientPipelineStatus.UNKNOWN, P._], () => <CustomChip rounded size='small' skin='light' label={state[0]} />)
    .with([ClientPipelineStatus.INACTIVE, true], () => (
      <CustomChip rounded size='small' skin='light' label={state[0]} />
    ))
    .with([ClientPipelineStatus.INACTIVE, false], () => (
      <CustomChip rounded size='small' skin='light' color='info' label='Compiling' />
    ))
    .with([ClientPipelineStatus.INITIALIZING, P._], () => (
      <CustomChip rounded size='small' skin='light' color='secondary' label={state[0]} />
    ))
    .with([ClientPipelineStatus.PROVISIONING, P._], () => (
      <CustomChip rounded size='small' skin='light' color='secondary' label={state[0]} />
    ))
    .with([ClientPipelineStatus.CREATE_FAILURE, P._], () => (
      <CustomChip rounded size='small' skin='light' color='error' label={state[0]} />
    ))
    .with([ClientPipelineStatus.STARTING, P._], () => (
      <CustomChip rounded size='small' skin='light' color='secondary' label={state[0]} />
    ))
    .with([ClientPipelineStatus.STARTUP_FAILURE, P._], () => (
      <CustomChip rounded size='small' skin='light' color='error' label={state[0]} />
    ))
    .with([ClientPipelineStatus.RUNNING, P._], () => (
      <CustomChip rounded size='small' skin='light' color='success' label={state[0]} />
    ))
    .with([ClientPipelineStatus.PAUSING, P._], () => (
      <CustomChip rounded size='small' skin='light' color='info' label={state[0]} />
    ))
    .with([ClientPipelineStatus.PAUSED, true], () => (
      <CustomChip rounded size='small' skin='light' color='info' label={state[0]} />
    ))
    .with([ClientPipelineStatus.PAUSED, false], () => (
      <CustomChip rounded size='small' skin='light' color='info' label='Compiling' />
    ))
    .with([ClientPipelineStatus.FAILED, P._], () => (
      <Tooltip title={params.row.state.error?.message || 'Unknown Error'} disableInteractive>
        <CustomChip
          rounded
          size='small'
          skin='light'
          color='error'
          label={state[0]}
          onDelete={() => shutdownPipelineClick(params.row.descriptor.pipeline_id)}
        />
      </Tooltip>
    ))
    .with([ClientPipelineStatus.SHUTTING_DOWN, P._], () => (
      <CustomChip rounded size='small' skin='light' color='secondary' label={state[0]} />
    ))
    .exhaustive()

  return (
    <Badge badgeContent={params.row.warn_cnt} color='warning'>
      <Badge badgeContent={params.row.error_cnt} color='error'>
        {chip}
      </Badge>
    </Badge>
  )
}

const PipelineActions = (params: { row: Pipeline }) => {
  const pipeline = params.row.descriptor

  const state = usePipelineState(params)

  const startPipelineClick = useStartPipeline()
  const pausePipelineClick = usePausePipeline()
  const shutdownPipelineClick = useShutdownPipeline()
  const deletePipelineClick = useDeletePipeline()

  const actions = {
    pause: () => (
      <Tooltip title='Pause Pipeline'>
        <IconButton className='pauseButton' size='small' onClick={() => pausePipelineClick(pipeline.pipeline_id)}>
          <Icon icon='bx:pause-circle' fontSize={20} />
        </IconButton>
      </Tooltip>
    ),
    start: () => (
      <Tooltip title='Start Pipeline'>
        <IconButton className='startButton' size='small' onClick={() => startPipelineClick(pipeline.pipeline_id)}>
          <Icon icon='bx:play-circle' fontSize={20} />
        </IconButton>
      </Tooltip>
    ),
    spinner: () => (
      <Tooltip title={state[0]}>
        <IconButton size='small'>
          <Icon icon='svg-spinners:270-ring-with-bg' fontSize={20} />
        </IconButton>
      </Tooltip>
    ),
    shutdown: () => (
      <Tooltip title='Shutdown Pipeline'>
        <IconButton className='shutdownButton' size='small' onClick={() => shutdownPipelineClick(pipeline.pipeline_id)}>
          <Icon icon='bx:stop-circle' fontSize={20} />
        </IconButton>
      </Tooltip>
    ),
    inspect: () => (
      <Tooltip title='Inspect'>
        <IconButton size='small' component={Link} href='#'>
          <Icon icon='bx:show' fontSize={20} />
        </IconButton>
      </Tooltip>
    ),
    edit: () => (
      <Tooltip title='Edit Pipeline'>
        <IconButton
          className='editButton'
          size='small'
          href={`/streaming/builder/?pipeline_id=${pipeline.pipeline_id}`}
        >
          <Icon icon='bx:pencil' fontSize={20} />
        </IconButton>
      </Tooltip>
    ),
    delete: () => (
      <Tooltip title='Delete Pipeline'>
        <IconButton className='deleteButton' size='small' onClick={() => deletePipelineClick(pipeline.pipeline_id)}>
          <Icon icon='bx:trash-alt' fontSize={20} />
        </IconButton>
      </Tooltip>
    )
  }

  const enabled = match(state)
    .returnType<(keyof typeof actions)[]>()
    .with([ClientPipelineStatus.INACTIVE, true], () => ['start', 'edit', 'delete'])
    .with([ClientPipelineStatus.INACTIVE, false], () => ['edit', 'delete'])
    .with([ClientPipelineStatus.PROVISIONING, P._], () => ['spinner', 'edit'])
    .with([ClientPipelineStatus.INITIALIZING, P._], () => ['spinner', 'edit'])
    .with([ClientPipelineStatus.STARTING, P._], () => ['spinner', 'edit'])
    .with([ClientPipelineStatus.RUNNING, P._], () => ['pause', 'shutdown', 'edit'])
    .with([ClientPipelineStatus.PAUSING, P._], () => ['spinner', 'edit'])
    .with([ClientPipelineStatus.PAUSED, true], () => ['start', 'shutdown', 'edit'])
    .with([ClientPipelineStatus.SHUTTING_DOWN, P._], () => ['spinner', 'edit'])
    .with([ClientPipelineStatus.FAILED, P._], () => ['shutdown', 'edit'])
    .otherwise(() => ['edit'])

  return (
    <>
      {/* the className attributes are used by webui-tester */}
      {enabled.map(e => actions[e]())}
    </>
  )
}
