from typing import TYPE_CHECKING, Any, Dict, List, Type, TypeVar, Union

from attrs import define, field

from ..types import UNSET, Unset

if TYPE_CHECKING:
    from ..models.attached_connector import AttachedConnector
    from ..models.runtime_config import RuntimeConfig


T = TypeVar("T", bound="NewPipelineRequest")


@define
class NewPipelineRequest:
    """Request to create a new pipeline.

    Attributes:
        config (RuntimeConfig): Global pipeline configuration settings.
        description (str): Config description.
        name (str): Config name.
        connectors (Union[Unset, None, List['AttachedConnector']]): Attached connectors.
        program_id (Union[Unset, None, str]): Unique program id.
    """

    config: "RuntimeConfig"
    description: str
    name: str
    connectors: Union[Unset, None, List["AttachedConnector"]] = UNSET
    program_id: Union[Unset, None, str] = UNSET
    additional_properties: Dict[str, Any] = field(init=False, factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        config = self.config.to_dict()

        description = self.description
        name = self.name
        connectors: Union[Unset, None, List[Dict[str, Any]]] = UNSET
        if not isinstance(self.connectors, Unset):
            if self.connectors is None:
                connectors = None
            else:
                connectors = []
                for connectors_item_data in self.connectors:
                    connectors_item = connectors_item_data.to_dict()

                    connectors.append(connectors_item)

        program_id = self.program_id

        field_dict: Dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update(
            {
                "config": config,
                "description": description,
                "name": name,
            }
        )
        if connectors is not UNSET:
            field_dict["connectors"] = connectors
        if program_id is not UNSET:
            field_dict["program_id"] = program_id

        return field_dict

    @classmethod
    def from_dict(cls: Type[T], src_dict: Dict[str, Any]) -> T:
        from ..models.attached_connector import AttachedConnector
        from ..models.runtime_config import RuntimeConfig

        d = src_dict.copy()
        config = RuntimeConfig.from_dict(d.pop("config"))

        description = d.pop("description")

        name = d.pop("name")

        connectors = []
        _connectors = d.pop("connectors", UNSET)
        for connectors_item_data in _connectors or []:
            connectors_item = AttachedConnector.from_dict(connectors_item_data)

            connectors.append(connectors_item)

        program_id = d.pop("program_id", UNSET)

        new_pipeline_request = cls(
            config=config,
            description=description,
            name=name,
            connectors=connectors,
            program_id=program_id,
        )

        new_pipeline_request.additional_properties = d
        return new_pipeline_request

    @property
    def additional_keys(self) -> List[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
