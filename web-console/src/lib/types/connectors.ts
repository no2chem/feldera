export enum ConnectorType {
  KAFKA_IN = 'KafkaIn',
  KAFKA_OUT = 'KafkaOut',
  URL = 'HTTP GET',
  UNKNOWN = 'Unknown'
}

export enum Direction {
  INPUT,
  OUTPUT,
  INPUT_OUTPUT
}
