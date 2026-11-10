import { useId } from "react";
import type { AvatarSpec } from "../../contracts/generated/AvatarSpec";
import {
  ACCESSORIES,
  BACKGROUNDS,
  EYES,
  HAIRS,
  HAIR_COLORS,
  HEADS,
  MOUTHS,
  SKINS,
  type Swatch,
} from "./avatarCatalog";
import { PixelAvatar } from "./PixelAvatar";

interface Props {
  value: AvatarSpec;
  onChange: (value: AvatarSpec) => void;
  /** Nombre para la vista previa (etiqueta accesible). */
  name: string;
}

/**
 * Editor por piezas: formas con listas desplegables y colores con grupos de opciones
 * (navegables con flechas). Todo se puede hacer con teclado; cada color tiene nombre.
 */
export function AvatarEditor({ value, onChange, name }: Props) {
  const id = useId();
  const set = (part: keyof AvatarSpec) => (index: number) => {
    onChange({ ...value, [part]: index });
  };

  return (
    <div className="avatar-editor">
      <div className="avatar-editor__preview">
        <PixelAvatar spec={value} name={name || "sin nombre"} size={96} />
      </div>
      <div className="avatar-editor__controls">
        <PartSelect
          id={`${id}-head`}
          label="Cabeza"
          options={HEADS}
          value={value.head}
          onChange={set("head")}
        />
        <PartSelect
          id={`${id}-eyes`}
          label="Ojos"
          options={EYES}
          value={value.eyes}
          onChange={set("eyes")}
        />
        <PartSelect
          id={`${id}-mouth`}
          label="Boca"
          options={MOUTHS}
          value={value.mouth}
          onChange={set("mouth")}
        />
        <PartSelect
          id={`${id}-hair`}
          label="Pelo o tocado"
          options={HAIRS}
          value={value.hair}
          onChange={set("hair")}
        />
        <PartSelect
          id={`${id}-accessory`}
          label="Accesorio"
          options={ACCESSORIES}
          value={value.accessory}
          onChange={set("accessory")}
        />
        <SwatchGroup
          name={`${id}-skin`}
          legend="Piel o material"
          swatches={SKINS}
          value={value.skin}
          onChange={set("skin")}
        />
        <SwatchGroup
          name={`${id}-hair-color`}
          legend="Color del pelo o tocado"
          swatches={HAIR_COLORS}
          value={value.hairColor}
          onChange={set("hairColor")}
        />
        <SwatchGroup
          name={`${id}-background`}
          legend="Fondo"
          swatches={BACKGROUNDS}
          value={value.background}
          onChange={set("background")}
        />
      </div>
    </div>
  );
}

function PartSelect({
  id,
  label,
  options,
  value,
  onChange,
}: {
  id: string;
  label: string;
  options: string[];
  value: number;
  onChange: (index: number) => void;
}) {
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      <select
        id={id}
        value={value}
        onChange={(event) => {
          onChange(Number(event.target.value));
        }}
      >
        {options.map((option, index) => (
          <option key={option} value={index}>
            {option}
          </option>
        ))}
      </select>
    </div>
  );
}

function SwatchGroup({
  name,
  legend,
  swatches,
  value,
  onChange,
}: {
  name: string;
  legend: string;
  swatches: Swatch[];
  value: number;
  onChange: (index: number) => void;
}) {
  return (
    <fieldset className="swatches">
      <legend>
        {legend}:{" "}
        <span className="swatches__current">{swatches[value]?.name}</span>
      </legend>
      {swatches.map((swatch, index) => (
        <label
          key={swatch.name}
          className="swatches__option"
          title={swatch.name}
        >
          <input
            type="radio"
            className="visually-hidden"
            name={name}
            value={index}
            checked={value === index}
            onChange={() => {
              onChange(index);
            }}
          />
          <span
            className="swatches__chip"
            aria-hidden="true"
            data-color={swatch.color}
            ref={(element) => {
              // Color por CSSOM (no atributo style en línea): compatible con la CSP estricta.
              if (element) element.style.backgroundColor = swatch.color;
            }}
          />
          <span className="visually-hidden">{swatch.name}</span>
        </label>
      ))}
    </fieldset>
  );
}
